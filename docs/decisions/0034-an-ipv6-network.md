# 0034. An IPv6 network behind the network card

- **Status:** Proposed
- **Date:** 2026-11-09
- **Amends:** [0033](0033-the-network-card-and-bulk-transfers.md), decision 2 (what
  the network behind the card answers)

## Context

NeOS's second networking stage is IPv6, before IPv4
([NeOS ADR 0062](https://github.com/MattA-Official/NeOS/blob/main/docs/decisions/0062-ipv6-first.md)).
Its network service gives each card a link-local address and one from each prefix a
router advertises, pings, and sends UDP. `cargo xtask test`'s `network` scenario now
runs QEMU with `-netdev user,id=net0,ipv4=off,ipv6=on,restrict=on`, and checks that
the card gets an address in `fec0::/64`, that the router is `fe80::2`, and that
`fec0::2` answers pings; the UDP it sends goes through `::1`, inside NeOS. The root
task's ARP request is gone.

What NeOS now sends to the network, and what QEMU 8.2's slirp (libslirp 4.7,
`src/ip6_icmp.c`, `ip6_input.c` and `ndp_table.c`) does with each:

- **MLDv2 reports** to `ff02::16`, behind a hop-by-hop header with the router alert.
  slirp's IPv6 input takes only ICMPv6, UDP and TCP as the next header, so drops them.
- **Duplicate Address Detection**: Neighbour Solicitations from `::` for NeOS's own
  tentative addresses. slirp answers a solicitation only for its own addresses, the
  router's and the DNS server's (`::2` and `::3`, under `fec0::/64` and under
  `fe80::/64`), so says nothing, and DAD passes.
- **Router Solicitations** to `ff02::2`, with a hop limit of 255. slirp records the
  sender's MAC address in its neighbour table and multicasts a Router Advertisement
  to `ff02::1` from `fe80::2`: hop limit 64, no M or O flag, a router lifetime of
  1800 s, reachable time and retransmit timer 0; a source link-layer option
  with 52:56:00:00:00:02 (slirp makes a host's MAC address from 52:56 and the last
  four bytes of its IPv6 address); and the prefix `fec0::/64` with L and A, valid
  for 86 400 s and preferred for 14 400 s. It adds an RDNSS option for `fec0::3`
  only if the machine QEMU runs on has an IPv6 DNS server, so NeOS's test does not
  look for one.
- **Neighbour Solicitations** for `fec0::2` or `fe80::2`. slirp records the sender
  and answers with a Neighbour Advertisement from the target, with R, S and O, and
  a target link-layer option, to the sender (or to `ff02::1` from `::`).
- **Neighbour Advertisements**, answering slirp's own solicitations. slirp records
  the sender.
- **Echo requests** to `fec0::2`. slirp sends the same message back as an echo reply,
  from the address asked. To send anything unicast it needs the destination's MAC
  address: if its table lacks it, it solicits it, from `fec0::2` to the solicited-node
  group, and sends the reply once the advertisement comes.

Every packet slirp sends has a hop limit of 255. Nebu's network answers only ARP, and
halts on any other frame (Nebu ADR 0033), so no NeOS with the network service runs in
it.

## Options

### What Nebu's network does with IPv6

- **Ignore it.** NeOS would get no addresses beyond link-local, and its test would
  fail in Nebu, which is to run NeOS's tests as QEMU does.
- **A copy of slirp's IPv6**: the above, and UDP and TCP to the outside world. NeOS's
  test sends neither, and with `restrict=on` QEMU forwards neither, so Nebu would
  model what nothing uses.
- **The ICMPv6 that slirp answers, and nothing more** _(chosen)_: Router and
  Neighbour Solicitations answered for slirp's two hosts, echo requests to them
  answered, slirp's neighbour table with its solicitation for an unknown address,
  and what slirp drops dropped. IPv6 UDP and TCP, and echo requests to any other
  address, which slirp would send outside, halt Nebu.

### Whether the Router Advertisement names a DNS server

- **Always**, as on a machine with IPv6 DNS. Then NeOS's test would see a DNS server in
  Nebu that it might not in QEMU.
- **Never** _(chosen)_, as on a machine without, which is the case on GitHub's
  runners. NeOS's test accepts either.

### Unsolicited Router Advertisements

slirp also multicasts one every 200 to 600 seconds. Every scenario finishes in under
two minutes, and NeOS solicits one when its link comes up, so **Nebu sends none**
_(chosen)_.

## Decision

1. Nebu's network answers IPv6 as slirp does, for what NeOS sends: Router
   Solicitations with a Router Advertisement, as above, without RDNSS; Neighbour
   Solicitations for `fec0::2`, `fec0::3`, `fe80::2` or `fe80::3` with a Neighbour
   Advertisement; and echo requests to those addresses with an echo reply. Each is
   checked as slirp checks it: hop limit 255 for Neighbour Discovery, a code of 0, and
   a correct checksum.
2. It keeps slirp's neighbour table: a sender's MAC address is recorded from a Router
   Solicitation, a Neighbour Solicitation from an address other than `::`, and a
   Neighbour Advertisement. A reply to an address it lacks waits while a Neighbour
   Solicitation for it goes out, and goes once the advertisement comes.
3. It drops what slirp drops: packets with a next header other than ICMPv6, UDP or
   TCP (MLD's hop-by-hop header), other ICMPv6 messages, and Neighbour Solicitations
   for other addresses. ARP is answered as before.
4. It halts on IPv6 UDP and TCP, on echo requests to other addresses, and on any frame
   that is neither ARP nor IPv6.

## Consequences

- NeOS's `network` scenario runs in Nebu as in QEMU, with the same lines out.
- A NeOS that sends UDP or TCP off the machine halts Nebu with a message naming it,
  for the stage that does to model.
- Nebu checks ICMPv6 checksums, so a NeOS that gets one wrong sees no answer, as in
  QEMU.
- NeOS's SSH stage needs a forwarded TCP port; that is Nebu ADR 0035.
