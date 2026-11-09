//! The IPv6 side of the network behind Nebu's network card: what QEMU 8.2's slirp
//! (libslirp 4.7) answers, as far as NeOS uses it (ADR 0034).
//!
//! slirp is a router at `fe80::2` that advertises `fec0::/64`, and two hosts, the
//! router and the DNS server, `::2` and `::3` under both prefixes. It answers Router
//! Solicitations, Neighbour Solicitations for its hosts and echo requests to them, and
//! keeps a neighbour table, from which it finds the MAC address to send a reply to,
//! soliciting it first if it has none. What slirp drops is dropped here; what slirp
//! would send to the world outside halts Nebu.

/// slirp's prefix, `fec0::/64`, and the link-local one, `fe80::/64`.
const PREFIX: [u8; 8] = [0xFE, 0xC0, 0, 0, 0, 0, 0, 0];
const LINK_LOCAL: [u8; 8] = [0xFE, 0x80, 0, 0, 0, 0, 0, 0];
/// The router's interface identifier, and the DNS server's.
const ROUTER: u8 = 2;
const DNS: u8 = 3;
/// `ff02::1`, every node on the link.
const ALL_NODES: [u8; 16] = [0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
/// IPv6's EtherType, and the next headers that matter.
pub const ETHER_TYPE: [u8; 2] = [0x86, 0xDD];
const ICMPV6: u8 = 58;
const UDP: u8 = 17;
const TCP: u8 = 6;
/// The ICMPv6 messages slirp handles.
const ECHO_REQUEST: u8 = 128;
const ECHO_REPLY: u8 = 129;
const ROUTER_SOLICITATION: u8 = 133;
const ROUTER_ADVERTISEMENT: u8 = 134;
const NEIGHBOUR_SOLICITATION: u8 = 135;
const NEIGHBOUR_ADVERTISEMENT: u8 = 136;
/// The hop limit slirp sends with, and that Neighbour Discovery needs.
const HOP_LIMIT: u8 = 255;
/// The Router Advertisement's fields: hop limit 64, a router lifetime of 1800 s, and
/// the prefix valid for 86 400 s and preferred for 14 400 s.
const ADVERTISED_HOP_LIMIT: u8 = 64;
const ROUTER_LIFETIME: u16 = 1800;
const VALID_LIFETIME: u32 = 86_400;
const PREFERRED_LIFETIME: u32 = 14_400;
/// The prefix information option's L and A flags.
const ON_LINK_AUTONOMOUS: u8 = 0xC0;
/// The Neighbour Advertisement's R, S and O flags.
const ROUTER_FLAG: u8 = 0x80;
const SOLICITED_FLAG: u8 = 0x40;
const OVERRIDE_FLAG: u8 = 0x20;

/// slirp's neighbour table, and the packets waiting for an address to go into it.
#[derive(Default)]
pub struct Slirp6 {
    neighbours: Vec<([u8; 16], [u8; 6])>,
    waiting: Vec<Vec<u8>>,
}

impl Slirp6 {
    /// Takes an IPv6 frame from the card, and returns the frames the network sends
    /// back, or why Nebu must halt.
    pub fn frame(&mut self, frame: &[u8]) -> Result<Vec<Vec<u8>>, &'static str> {
        let mut out = Vec::new();
        let Some(packet) = frame.get(14..) else {
            return Ok(out);
        };
        if packet.len() < 40 || packet[0] >> 4 != 6 {
            return Ok(out);
        }
        let length = usize::from(u16::from_be_bytes([packet[4], packet[5]]));
        let Some(packet) = packet.get(..40 + length) else {
            return Ok(out);
        };
        let (next, hop_limit) = (packet[6], packet[7]);
        let source: [u8; 16] = packet[8..24].try_into().expect("16 bytes");
        let destination: [u8; 16] = packet[24..40].try_into().expect("16 bytes");
        let body = &packet[40..];
        match next {
            ICMPV6 => {}
            UDP => {
                return Err("USB: IPv6 UDP sent to Nebu's network, which does not model the \
                            world outside slirp");
            }
            TCP => {
                return Err("USB: IPv6 TCP sent to Nebu's network, which does not model the \
                            world outside slirp");
            }
            // slirp takes only ICMPv6, UDP and TCP, so drops MLD behind its
            // hop-by-hop header.
            _ => return Ok(out),
        }
        if body.len() < 4 || checksum(&source, &destination, body) != 0 {
            return Ok(out);
        }
        let sender: [u8; 6] = frame[6..12].try_into().expect("6 bytes");
        let (kind, code) = (body[0], body[1]);
        let neighbour_discovery = hop_limit == HOP_LIMIT && code == 0;
        match kind {
            ECHO_REQUEST if is_host(&destination) => {
                let mut reply = packet.to_vec();
                reply[8..24].copy_from_slice(&destination);
                reply[24..40].copy_from_slice(&source);
                reply[40] = ECHO_REPLY;
                self.output(reply, &mut out);
            }
            ECHO_REQUEST => {
                return Err("USB: an IPv6 echo request to an address outside slirp, which \
                            Nebu's network does not model");
            }
            ROUTER_SOLICITATION if neighbour_discovery && body.len() >= 8 => {
                self.learn(source, sender, &mut out);
                self.output(router_advertisement(), &mut out);
            }
            NEIGHBOUR_SOLICITATION if neighbour_discovery && body.len() >= 24 => {
                let target: [u8; 16] = body[8..24].try_into().expect("16 bytes");
                let from_nowhere = source == [0; 16];
                let to_group = destination[..13] == solicited_node(&[0; 16])[..13];
                if target[0] != 0xFF && (!from_nowhere || to_group) && is_host(&target) {
                    self.learn(source, sender, &mut out);
                    let to = if from_nowhere { ALL_NODES } else { source };
                    self.output(neighbour_advertisement(target, to), &mut out);
                }
            }
            NEIGHBOUR_ADVERTISEMENT if neighbour_discovery && body.len() >= 24 => {
                let solicited = body[4] & SOLICITED_FLAG != 0;
                if body[8] != 0xFF && (destination[0] != 0xFF || !solicited) {
                    self.learn(source, sender, &mut out);
                }
            }
            _ => {}
        }
        Ok(out)
    }

    /// Records that `address` is at `mac`, as slirp's table does, but for multicast
    /// and unspecified addresses, and sends what waited for it.
    fn learn(&mut self, address: [u8; 16], mac: [u8; 6], out: &mut Vec<Vec<u8>>) {
        if address[0] == 0xFF || address == [0; 16] {
            return;
        }
        match self.neighbours.iter_mut().find(|(known, _)| *known == address) {
            Some(entry) => entry.1 = mac,
            None => self.neighbours.push((address, mac)),
        }
        let (ready, still): (Vec<_>, Vec<_>) =
            self.waiting.drain(..).partition(|packet| packet[24..40] == address);
        self.waiting = still;
        for packet in ready {
            out.push(ethernet(mac, &packet));
        }
    }

    /// Sends `packet`, as slirp's `ip6_output` does: with its hop limit 255 and its
    /// traffic class and flow label 0, and its checksum made. A multicast goes to its
    /// group's MAC address; a unicast to the neighbour's, or, if slirp does not know
    /// it, waits while a Neighbour Solicitation asks.
    fn output(&mut self, mut packet: Vec<u8>, out: &mut Vec<Vec<u8>>) {
        packet[..4].copy_from_slice(&[0x60, 0, 0, 0]);
        packet[7] = HOP_LIMIT;
        packet[42..44].fill(0);
        let source: [u8; 16] = packet[8..24].try_into().expect("16 bytes");
        let destination: [u8; 16] = packet[24..40].try_into().expect("16 bytes");
        let sum = checksum(&source, &destination, &packet[40..]);
        packet[42..44].copy_from_slice(&sum.to_be_bytes());
        if destination[0] == 0xFF {
            let group =
                [0x33, 0x33, destination[12], destination[13], destination[14], destination[15]];
            out.push(ethernet(group, &packet));
        } else if let Some(&(_, mac)) =
            self.neighbours.iter().find(|(known, _)| *known == destination)
        {
            out.push(ethernet(mac, &packet));
        } else {
            self.waiting.push(packet);
            self.output(neighbour_solicitation(destination), out);
        }
    }
}

/// Whether `address` is one of slirp's hosts: the router or the DNS server, under
/// its prefix or the link-local one.
fn is_host(address: &[u8; 16]) -> bool {
    (address[..8] == PREFIX || address[..8] == LINK_LOCAL)
        && address[8..15] == [0; 7]
        && (address[15] == ROUTER || address[15] == DNS)
}

/// The MAC address slirp gives one of its addresses: 52:56 and its last four bytes.
fn mac_of(address: &[u8; 16]) -> [u8; 6] {
    [0x52, 0x56, address[12], address[13], address[14], address[15]]
}

/// `address`'s solicited-node group, `ff02::1:ffXX:XXXX`.
fn solicited_node(address: &[u8; 16]) -> [u8; 16] {
    let mut group = [0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xFF, 0, 0, 0];
    group[13..].copy_from_slice(&address[13..]);
    group
}

/// An IPv6 packet from `source` to `destination` carrying the ICMPv6 message
/// `message`, its checksum still to be made.
fn icmp_packet(source: [u8; 16], destination: [u8; 16], message: &[u8]) -> Vec<u8> {
    let mut packet = vec![0x60, 0, 0, 0];
    packet.extend_from_slice(&(message.len() as u16).to_be_bytes());
    packet.extend_from_slice(&[ICMPV6, HOP_LIMIT]);
    packet.extend_from_slice(&source);
    packet.extend_from_slice(&destination);
    packet.extend_from_slice(message);
    packet
}

/// slirp's Router Advertisement, to every node, from `fe80::2`.
fn router_advertisement() -> Vec<u8> {
    let mut router = [0; 16];
    router[..8].copy_from_slice(&LINK_LOCAL);
    router[15] = ROUTER;
    let mut message = vec![ROUTER_ADVERTISEMENT, 0, 0, 0, ADVERTISED_HOP_LIMIT, 0];
    message.extend_from_slice(&ROUTER_LIFETIME.to_be_bytes());
    message.extend_from_slice(&[0; 8]);
    message.extend_from_slice(&[1, 1]);
    message.extend_from_slice(&mac_of(&router));
    message.extend_from_slice(&[3, 4, 64, ON_LINK_AUTONOMOUS]);
    message.extend_from_slice(&VALID_LIFETIME.to_be_bytes());
    message.extend_from_slice(&PREFERRED_LIFETIME.to_be_bytes());
    message.extend_from_slice(&[0; 4]);
    message.extend_from_slice(&PREFIX);
    message.extend_from_slice(&[0; 8]);
    icmp_packet(router, ALL_NODES, &message)
}

/// slirp's Neighbour Advertisement for its host `target`, to `to`.
fn neighbour_advertisement(target: [u8; 16], to: [u8; 16]) -> Vec<u8> {
    let solicited = if to[0] == 0xFF { 0 } else { SOLICITED_FLAG };
    let mut message = vec![NEIGHBOUR_ADVERTISEMENT, 0, 0, 0];
    message.extend_from_slice(&[ROUTER_FLAG | solicited | OVERRIDE_FLAG, 0, 0, 0]);
    message.extend_from_slice(&target);
    message.extend_from_slice(&[2, 1]);
    message.extend_from_slice(&mac_of(&target));
    icmp_packet(target, to, &message)
}

/// slirp's Neighbour Solicitation for `address`, from the router's address under its
/// prefix.
fn neighbour_solicitation(address: [u8; 16]) -> Vec<u8> {
    let mut host = [0; 16];
    host[..8].copy_from_slice(&PREFIX);
    host[15] = ROUTER;
    let mut message = vec![NEIGHBOUR_SOLICITATION, 0, 0, 0, 0, 0, 0, 0];
    message.extend_from_slice(&address);
    message.extend_from_slice(&[1, 1]);
    message.extend_from_slice(&mac_of(&host));
    icmp_packet(host, solicited_node(&address), &message)
}

/// An Ethernet frame with `packet`, to `destination`, from the MAC address slirp
/// gives the packet's source.
fn ethernet(destination: [u8; 6], packet: &[u8]) -> Vec<u8> {
    let source: [u8; 16] = packet[8..24].try_into().expect("16 bytes");
    let mut frame = destination.to_vec();
    frame.extend_from_slice(&mac_of(&source));
    frame.extend_from_slice(&ETHER_TYPE);
    frame.extend_from_slice(packet);
    frame
}

/// The ICMPv6 checksum of `message` between `source` and `destination`: 0 when the
/// message's own checksum is right, or the one to put in it when that is 0.
fn checksum(source: &[u8; 16], destination: &[u8; 16], message: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut add = |bytes: &[u8]| {
        for pair in bytes.chunks(2) {
            let high = u32::from(pair[0]) << 8;
            sum += high | pair.get(1).map_or(0, |&low| u32::from(low));
        }
    };
    add(source);
    add(destination);
    add(&(message.len() as u32).to_be_bytes());
    add(&[0, 0, 0, ICMPV6]);
    add(message);
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: [u8; 6] = [0x40, 0x54, 0, 0x12, 0x34, 0x57];
    const OWN: [u8; 16] = [0xFE, 0xC0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8];
    const OWN_LINK: [u8; 16] = [0xFE, 0x80, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8];
    const HOST: [u8; 16] = [0xFE, 0xC0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
    const ROUTER_LINK: [u8; 16] = [0xFE, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];

    /// A frame from the card with an ICMPv6 message, its checksum made.
    fn sent(source: [u8; 16], destination: [u8; 16], hop_limit: u8, message: &[u8]) -> Vec<u8> {
        let mut packet = icmp_packet(source, destination, message);
        packet[7] = hop_limit;
        let sum = checksum(&source, &destination, &packet[40..]);
        packet[42..44].copy_from_slice(&sum.to_be_bytes());
        let mut frame = vec![0x33, 0x33, 0, 0, 0, 2];
        frame.extend_from_slice(&CARD);
        frame.extend_from_slice(&ETHER_TYPE);
        frame.extend_from_slice(&packet);
        frame
    }

    fn solicitation(source: [u8; 16], target: [u8; 16]) -> Vec<u8> {
        let mut message = vec![NEIGHBOUR_SOLICITATION, 0, 0, 0, 0, 0, 0, 0];
        message.extend_from_slice(&target);
        if source != [0; 16] {
            message.extend_from_slice(&[1, 1]);
            message.extend_from_slice(&CARD);
        }
        let to = solicited_node(&target);
        sent(source, to, HOP_LIMIT, &message)
    }

    fn is_valid(frame: &[u8]) -> bool {
        let packet = &frame[14..];
        let source: [u8; 16] = packet[8..24].try_into().unwrap();
        let destination: [u8; 16] = packet[24..40].try_into().unwrap();
        packet[7] == HOP_LIMIT && checksum(&source, &destination, &packet[40..]) == 0
    }

    #[test]
    fn answers_a_router_solicitation() {
        let mut network = Slirp6::default();
        let rs = sent(
            OWN_LINK,
            [0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2],
            255,
            &[ROUTER_SOLICITATION, 0, 0, 0, 0, 0, 0, 0],
        );
        let out = network.frame(&rs).unwrap();
        assert_eq!(out.len(), 1);
        let ra = &out[0];
        assert_eq!(ra[..6], [0x33, 0x33, 0, 0, 0, 1]);
        assert_eq!(ra[6..12], [0x52, 0x56, 0, 0, 0, 2]);
        assert_eq!(ra[14 + 8..14 + 24], ROUTER_LINK);
        assert_eq!(ra[14 + 40], ROUTER_ADVERTISEMENT);
        assert_eq!(ra.len(), 14 + 40 + 16 + 8 + 32);
        assert!(is_valid(ra));
        // A hop limit other than 255 is not answered.
        let mut low = rs.clone();
        low[14 + 7] = 64;
        assert!(network.frame(&low).unwrap().is_empty());
    }

    #[test]
    fn answers_solicitations_only_for_its_hosts() {
        let mut network = Slirp6::default();
        // Duplicate Address Detection gets no answer.
        assert!(network.frame(&solicitation([0; 16], OWN)).unwrap().is_empty());
        let out = network.frame(&solicitation(OWN, HOST)).unwrap();
        assert_eq!(out.len(), 1);
        let na = &out[0];
        assert_eq!(na[..6], CARD);
        assert_eq!(na[14 + 40], NEIGHBOUR_ADVERTISEMENT);
        assert_eq!(na[14 + 44], ROUTER_FLAG | SOLICITED_FLAG | OVERRIDE_FLAG);
        assert_eq!(na[14 + 48..14 + 64], HOST);
        assert_eq!(na[14 + 66..14 + 72], [0x52, 0x56, 0, 0, 0, 2]);
        assert!(is_valid(na));
    }

    #[test]
    fn answers_pings_once_it_knows_the_sender() {
        let mut network = Slirp6::default();
        let ping = sent(OWN, HOST, 64, &[ECHO_REQUEST, 0, 0, 0, 0x12, 0x34, 0, 1, b'h', b'i']);
        // It solicits the sender first.
        let out = network.frame(&ping).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0][14 + 40], NEIGHBOUR_SOLICITATION);
        assert_eq!(out[0][14 + 24..14 + 40], solicited_node(&OWN));
        assert!(is_valid(&out[0]));
        // The advertisement lets the reply go.
        let mut message =
            vec![NEIGHBOUR_ADVERTISEMENT, 0, 0, 0, SOLICITED_FLAG | OVERRIDE_FLAG, 0, 0, 0];
        message.extend_from_slice(&OWN);
        let out = network.frame(&sent(OWN, HOST, 255, &message)).unwrap();
        assert_eq!(out.len(), 1);
        let reply = &out[0];
        assert_eq!(reply[..6], CARD);
        assert_eq!(reply[14 + 8..14 + 24], HOST);
        assert_eq!(reply[14 + 24..14 + 40], OWN);
        assert_eq!(reply[14 + 40], ECHO_REPLY);
        assert_eq!(reply[14 + 44..], [0x12, 0x34, 0, 1, b'h', b'i']);
        assert!(is_valid(reply));
        // Now it knows, a ping is answered at once.
        assert_eq!(network.frame(&ping).unwrap()[0][14 + 40], ECHO_REPLY);
    }

    #[test]
    fn drops_what_slirp_drops_and_halts_on_the_rest() {
        let mut network = Slirp6::default();
        // MLD, behind a hop-by-hop header.
        let mut mld =
            sent(OWN_LINK, [0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x16], 1, &[0; 8]);
        mld[14 + 6] = 0;
        assert!(network.frame(&mld).unwrap().is_empty());
        // A bad checksum.
        let mut ping = sent(OWN, HOST, 64, &[ECHO_REQUEST, 0, 0, 0, 0, 1, 0, 1]);
        ping[14 + 42] ^= 1;
        assert!(network.frame(&ping).unwrap().is_empty());
        // Ping outside, and UDP.
        let outside = [0x20, 1, 0xD, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        assert!(
            network.frame(&sent(OWN, outside, 64, &[ECHO_REQUEST, 0, 0, 0, 0, 1, 0, 1])).is_err()
        );
        let mut udp = sent(OWN, HOST, 64, &[0; 8]);
        udp[14 + 6] = UDP;
        assert!(network.frame(&udp).is_err());
    }
}
