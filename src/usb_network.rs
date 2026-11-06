//! QEMU 8.2's USB network card, `usb-net`, in its CDC Ethernet (ECM) configuration,
//! on a network that answers as QEMU's user networking, slirp, does, as far as NeOS
//! uses them (ADR 0033).
//!
//! The card has QEMU's descriptors and strings, byte for byte: two configurations,
//! RNDIS's first, then CDC Ethernet's, whose data interface has its bulk endpoints
//! only in its second setting. Frames go out in bulk transfers that end with a short
//! packet, and come in the same way, one at a time, with a zero-length packet after
//! a frame that fills its last packet, as QEMU's `hw/usb/dev-network.c` does. The
//! network behind it is the gateway, 10.0.2.2, and the name server, 10.0.2.3, which
//! answer ARP requests as slirp does, with a 64-byte frame. Anything else sent halts
//! Nebu, as do RNDIS and the card's notifications, which NeOS does not use.

use std::collections::VecDeque;

use crate::usb_devices::{Answer, Reply, Token};

/// The address `cargo xtask` gives QEMU's card, and the one QEMU puts in its MAC
/// address string: the same, with 0x40 for the first byte.
const CARD_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x57];
/// The prefix of the address slirp answers with, which it ends with the IPv4
/// address asked for.
const SLIRP_MAC_PREFIX: [u8; 2] = [0x52, 0x55];
/// slirp's network, 10.0.2.0/24, its gateway and its name server.
const NETWORK: [u8; 3] = [10, 0, 2];
const GATEWAY: [u8; 4] = [10, 0, 2, 2];
const NAME_SERVER: [u8; 4] = [10, 0, 2, 3];
/// ARP's EtherType, and the parts of a request for an IPv4 address on Ethernet.
const ARP: [u8; 2] = [0x08, 0x06];
const ARP_ETHERNET_IPV4: [u8; 6] = [0, 1, 0x08, 0x00, 6, 4];
const ARP_REQUEST: u8 = 1;
const ARP_REPLY: u8 = 2;
/// How long slirp's ARP replies are: padded to Ethernet's shortest frame.
const ARP_REPLY_LENGTH: usize = 64;
/// How many bytes QEMU's card holds of a frame being sent.
const OUT_BUFFER: usize = 2048;
/// The bulk endpoints' largest packet, by which QEMU's card tells a frame's end.
const BULK_PACKET: usize = 64;

/// The configuration values: CDC Ethernet's, and RNDIS's.
const CDC_CONFIGURATION: u16 = 1;
const RNDIS_CONFIGURATION: u16 = 2;
/// CDC Ethernet's Set Ethernet Packet Filter.
const SET_PACKET_FILTER: u8 = 0x43;

/// The device descriptor: USB 2.0, the communications class, 64-byte packets on
/// endpoint 0, NetChip's RNDIS gadget (0525:a4a2), strings 1, 2 and 10, and two
/// configurations.
const DEVICE: [u8; 18] =
    [18, 1, 0x00, 0x02, 2, 0, 0, 64, 0x25, 0x05, 0xA2, 0xA4, 0, 0, 1, 2, 10, 2];
/// The RNDIS configuration, value 2, as QEMU lays it out.
const RNDIS: [u8; 67] = [
    9, 2, 67, 0, 2, 2, 9, 0xC0, 0x32, // configuration
    9, 4, 0, 0, 1, 2, 2, 0xFF, 6, // control interface
    5, 0x24, 0, 0x10, 0x01, // header
    5, 0x24, 1, 0, 1, // call management
    4, 0x24, 2, 0, // abstract control management
    5, 0x24, 6, 0, 1, // union
    7, 5, 0x81, 3, 16, 0, 32, // status endpoint
    9, 4, 1, 0, 2, 0x0A, 0, 0, 4, // data interface
    7, 5, 0x82, 2, 64, 0, 0, // bulk IN
    7, 5, 0x02, 2, 64, 0, 0, // bulk OUT
];
/// The CDC Ethernet configuration, value 1: its control interface's Ethernet
/// descriptor names string 3 for the MAC address and frames of 1514 bytes, and its
/// data interface's setting 0 has no endpoints.
const CDC: [u8; 80] = [
    9, 2, 80, 0, 2, 1, 7, 0xC0, 0x32, // configuration
    9, 4, 0, 0, 1, 2, 6, 0, 5, // control interface
    5, 0x24, 0, 0x10, 0x01, // header
    5, 0x24, 6, 0, 1, // union
    13, 0x24, 0x0F, 3, 0, 0, 0, 0, 0xEA, 0x05, 0, 0, 0, // Ethernet
    7, 5, 0x81, 3, 16, 0, 32, // status endpoint
    9, 4, 1, 0, 0, 0x0A, 0, 0, 0, // data interface, off
    9, 4, 1, 1, 2, 0x0A, 0, 0, 4, // data interface, on
    7, 5, 0x82, 2, 64, 0, 0, // bulk IN
    7, 5, 0x02, 2, 64, 0, 0, // bulk OUT
];
/// The strings, from 1. String 3, the MAC address, is made from [`CARD_MAC`].
const STRINGS: [&str; 10] = [
    "QEMU",
    "RNDIS/QEMU USB Network Device",
    "",
    "QEMU USB Net Data Interface",
    "QEMU USB Net Control Interface",
    "QEMU USB Net RNDIS Control Interface",
    "QEMU USB Net CDC",
    "QEMU USB Net Subset",
    "QEMU USB Net RNDIS",
    "1",
];

/// The card, and the network behind it.
#[derive(Default)]
pub struct Network {
    /// The configuration chosen, 0 for none, and the data interface's setting.
    configuration: u16,
    data_setting: u16,
    /// The frame being sent, so far.
    out: Vec<u8>,
    /// The frame being received, and how much of it has gone.
    incoming: Vec<u8>,
    taken: usize,
    /// The frames the network has sent that the card has not taken yet, oldest
    /// first, as slirp keeps them while the card's buffer is full.
    queue: VecDeque<Vec<u8>>,
}

impl Network {
    /// Resets the card, as a port reset does. QEMU's card keeps its buffers.
    pub fn reset(&mut self) {
        self.configuration = 0;
        self.data_setting = 0;
    }

    /// Whether a configuration has been chosen.
    pub fn configured(&self) -> bool {
        self.configuration != 0
    }

    /// Set Configuration: `Ok(false)` for a value the card does not have, which it
    /// stalls.
    pub fn configure(&mut self, value: u16) -> Result<bool, &'static str> {
        match value {
            0 | CDC_CONFIGURATION => {
                self.configuration = value;
                self.data_setting = 0;
                Ok(true)
            }
            RNDIS_CONFIGURATION => Err("USB: the network card's RNDIS, which Nebu does not model"),
            _ => Ok(false),
        }
    }

    /// Get Descriptor for `kind` number `index`, or `None` for a stall.
    pub fn descriptor(&self, kind: u8, index: u8) -> Option<Vec<u8>> {
        match (kind, index) {
            (1, 0) => Some(DEVICE.to_vec()),
            (2, 0) => Some(RNDIS.to_vec()),
            (2, 1) => Some(CDC.to_vec()),
            (3, 0) => Some(vec![4, 3, 0x09, 0x04]),
            (3, 3) => Some(string(&mac_string())),
            (3, index) => STRINGS.get(usize::from(index) - 1).map(|text| string(text)),
            _ => None,
        }
    }

    /// Set Interface: the control interface has one setting, the data interface
    /// two. `None` for a stall.
    pub fn set_interface(&mut self, interface: u16, setting: u16) -> Option<Vec<u8>> {
        match (self.configuration, interface, setting) {
            (CDC_CONFIGURATION, 0, 0) => Some(Vec::new()),
            (CDC_CONFIGURATION, 1, 0 | 1) => {
                self.data_setting = setting;
                Some(Vec::new())
            }
            _ => None,
        }
    }

    /// A class request: only Set Ethernet Packet Filter, which QEMU's card takes and
    /// ignores.
    pub fn class_request(
        &mut self,
        request_type: u8,
        request: u8,
    ) -> Result<Option<Vec<u8>>, &'static str> {
        match (request_type, request) {
            (0x21, SET_PACKET_FILTER) if self.configuration == CDC_CONFIGURATION => {
                Ok(Some(Vec::new()))
            }
            _ => Err("a USB network card request that Nebu does not model"),
        }
    }

    /// A packet to one of its endpoints besides endpoint 0.
    pub fn handle(&mut self, endpoint: u8, token: Token, data: &[u8], room: usize) -> Answer {
        if self.configuration != CDC_CONFIGURATION {
            return Err("a USB transfer to a network card not yet configured");
        }
        match (endpoint, token) {
            (1, Token::In) => {
                Err("USB: the network card's notifications, which Nebu does not model")
            }
            (2, _) if self.data_setting != 1 => {
                Err("USB: a bulk transfer to the network card before its data interface is on")
            }
            (2, Token::In) => Ok(self.data_in(room)),
            (2, Token::Out) => self.data_out(data),
            _ => Err("a USB transfer to an endpoint the device does not have"),
        }
    }

    /// Hands over up to `room` bytes of the frame being received, as QEMU does: a
    /// frame that fills its last packet is followed by a zero-length one.
    fn data_in(&mut self, room: usize) -> Reply {
        if self.incoming.is_empty() {
            return Reply::Nak;
        }
        let end = self.incoming.len().min(self.taken + room);
        let part = self.incoming[self.taken..end].to_vec();
        self.taken = end;
        if self.taken >= self.incoming.len()
            && (!self.incoming.len().is_multiple_of(BULK_PACKET) || part.is_empty())
        {
            self.incoming.clear();
            self.taken = 0;
            self.take_next();
        }
        Reply::Ack { actual: part.len(), data: part }
    }

    /// Takes a packet of a frame being sent; a short one ends the frame, which goes
    /// to the network.
    fn data_out(&mut self, data: &[u8]) -> Answer {
        if self.out.len() + data.len() > OUT_BUFFER {
            return Err("USB: a frame longer than QEMU's network card holds");
        }
        self.out.extend_from_slice(data);
        if !data.len().is_multiple_of(BULK_PACKET) || data.is_empty() {
            let frame = std::mem::take(&mut self.out);
            self.send(&frame)?;
        }
        Ok(Reply::Ack { actual: data.len(), data: Vec::new() })
    }

    /// The network's answer to a frame: an ARP request for the gateway or the name
    /// server is answered, as slirp answers it; other ARP is ignored, as slirp
    /// ignores it; anything else halts Nebu.
    fn send(&mut self, frame: &[u8]) -> Result<(), &'static str> {
        if frame.len() < 42 || frame[12..14] != ARP {
            return Err(
                "USB: a frame other than ARP sent to Nebu's network, which it does not model",
            );
        }
        if frame[14..20] != ARP_ETHERNET_IPV4 || frame[21] != ARP_REQUEST {
            return Ok(());
        }
        let (sender, sender_ip, target_ip) = (&frame[22..28], &frame[28..32], &frame[38..42]);
        let asked = target_ip == GATEWAY || target_ip == NAME_SERVER;
        if target_ip == sender_ip || target_ip[..3] != NETWORK || !asked {
            return Ok(());
        }
        let mut answerer = [0; 6];
        answerer[..2].copy_from_slice(&SLIRP_MAC_PREFIX);
        answerer[2..].copy_from_slice(target_ip);
        let mut reply = vec![0; ARP_REPLY_LENGTH];
        reply[..6].copy_from_slice(&frame[6..12]);
        reply[6..12].copy_from_slice(&answerer);
        reply[12..14].copy_from_slice(&ARP);
        reply[14..20].copy_from_slice(&ARP_ETHERNET_IPV4);
        reply[21] = ARP_REPLY;
        reply[22..28].copy_from_slice(&answerer);
        reply[28..32].copy_from_slice(target_ip);
        reply[32..38].copy_from_slice(sender);
        reply[38..42].copy_from_slice(sender_ip);
        self.queue.push_back(reply);
        self.take_next();
        Ok(())
    }

    /// Takes the next frame from the network, if the card's buffer is free.
    fn take_next(&mut self) {
        if self.incoming.is_empty()
            && let Some(frame) = self.queue.pop_front()
        {
            self.incoming = frame;
            self.taken = 0;
        }
    }
}

/// The MAC address string, as QEMU makes it: lowercase hexadecimal, with 0x40 for
/// the first byte.
fn mac_string() -> String {
    let mut text = String::from("40");
    for byte in &CARD_MAC[1..] {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// A string descriptor holding `text`, in UTF-16, as QEMU makes one from ASCII.
fn string(text: &str) -> Vec<u8> {
    let mut descriptor = vec![(2 + 2 * text.len()) as u8, 3];
    for byte in text.bytes() {
        descriptor.extend_from_slice(&[byte, 0]);
    }
    descriptor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured() -> Network {
        let mut network = Network::default();
        assert_eq!(network.configure(CDC_CONFIGURATION), Ok(true));
        assert_eq!(network.set_interface(1, 1), Some(Vec::new()));
        network
    }

    fn arp_request(target: [u8; 4]) -> Vec<u8> {
        let mut frame = vec![0xFF; 6];
        frame.extend_from_slice(&[0x40, 0x54, 0, 0x12, 0x34, 0x57]);
        frame.extend_from_slice(&ARP);
        frame.extend_from_slice(&ARP_ETHERNET_IPV4);
        frame.extend_from_slice(&[0, ARP_REQUEST, 0x40, 0x54, 0, 0x12, 0x34, 0x57, 10, 0, 2, 15]);
        frame.extend_from_slice(&[0; 6]);
        frame.extend_from_slice(&target);
        frame
    }

    #[test]
    fn its_descriptors_are_qemus() {
        let network = Network::default();
        assert_eq!(network.descriptor(2, 0).unwrap().len(), usize::from(RNDIS[2]));
        assert_eq!(network.descriptor(2, 1).unwrap().len(), usize::from(CDC[2]));
        assert_eq!(network.descriptor(2, 2), None);
        let mac = network.descriptor(3, 3).unwrap();
        assert_eq!(mac[0], 26);
        assert_eq!(&mac[2..6], b"4\x000\x00");
        assert_eq!(network.descriptor(3, 1).unwrap(), string("QEMU"));
    }

    #[test]
    fn the_gateway_answers_arp() {
        let mut network = configured();
        assert_eq!(network.handle(2, Token::In, &[], 64), Ok(Reply::Nak));
        let request = arp_request(GATEWAY);
        assert!(network.handle(2, Token::Out, &request, 64).is_ok());
        // 64 bytes, then the zero-length packet that ends them.
        let Ok(Reply::Ack { actual: 64, data }) = network.handle(2, Token::In, &[], 64) else {
            panic!("no reply");
        };
        assert_eq!(
            network.handle(2, Token::In, &[], 64),
            Ok(Reply::Ack { actual: 0, data: vec![] })
        );
        assert_eq!(network.handle(2, Token::In, &[], 64), Ok(Reply::Nak));
        assert_eq!(data[6..12], [0x52, 0x55, 10, 0, 2, 2]);
        assert_eq!(data[21], ARP_REPLY);
        assert_eq!(data[38..42], [10, 0, 2, 15]);
    }

    #[test]
    fn halts_on_what_it_does_not_model() {
        let mut network = Network::default();
        assert!(network.handle(2, Token::In, &[], 64).is_err());
        assert!(network.configure(RNDIS_CONFIGURATION).is_err());
        let mut network = configured();
        let mut frame = arp_request(GATEWAY);
        frame[13] = 0x00;
        assert!(network.handle(2, Token::Out, &frame, 64).is_err());
        assert!(network.handle(1, Token::In, &[], 16).is_err());
    }
}
