//! The USB devices behind the controller: a hub with a keyboard on its first port,
//! answering as QEMU 8.2's `usb-hub` and `usb-kbd` do (ADR 0024).
//!
//! QEMU puts a full-speed hub of its own between the controller and a keyboard, so
//! Nebu does too. Each device answers the control requests NeOS sends through the
//! same three stages as QEMU's USB core: a setup packet, data, and status. A request
//! that changes the device takes effect at the status stage, as SET_ADDRESS must.
//! Requests NeOS does not send halt Nebu, except a HID class request that HID does
//! not define, which a keyboard stalls, as QEMU's and real ones do. The keyboard
//! types what `--keys` lists, a press or release each time it is polled (ADR 0025).

use std::collections::VecDeque;

/// The packets a host sends, as the token that starts each.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Token {
    Setup,
    In,
    Out,
}

/// How a device answered a packet.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Reply {
    /// It took `actual` bytes, or sent `data`, of which `actual` is the length.
    Ack { actual: usize, data: Vec<u8> },
    /// It has nothing to send yet.
    Nak,
    /// It refused.
    Stall,
}

impl Reply {
    fn took(actual: usize) -> Reply {
        Reply::Ack { actual, data: Vec::new() }
    }

    fn sent(data: Vec<u8>) -> Reply {
        Reply::Ack { actual: data.len(), data }
    }
}

/// What a device does with a packet, or why Nebu cannot say.
pub type Answer = Result<Reply, &'static str>;

/// The standard requests, and the descriptors, NeOS uses.
const GET_STATUS: u8 = 0;
const CLEAR_FEATURE: u8 = 1;
const SET_FEATURE: u8 = 3;
const SET_ADDRESS: u8 = 5;
const GET_DESCRIPTOR: u8 = 6;
const SET_CONFIGURATION: u8 = 9;
const DEVICE: u8 = 1;
const CONFIGURATION: u8 = 2;
const HUB: u8 = 0x29;
const REPORT: u8 = 0x22;
/// The bit of a request type that says the data goes to the host.
const IN: u8 = 0x80;

/// A hub port's status bits and change bits, and the features NeOS sets and
/// clears, as the USB 2.0 specification's chapter 11 numbers them.
const PORT_CONNECTION: u16 = 1 << 0;
const PORT_ENABLE: u16 = 1 << 1;
const PORT_RESET: u16 = 1 << 4;
const PORT_POWER: u16 = 1 << 8;
const FEATURE_ENABLE: u16 = 1;
const FEATURE_RESET: u16 = 4;
const FEATURE_POWER: u16 = 8;
const FEATURE_CHANGES: std::ops::RangeInclusive<u16> = 16..=20;

/// How many ports QEMU's hub has.
const PORTS: usize = 8;
/// The largest control transfer a QEMU device takes.
const LARGEST_CONTROL: usize = 4096;

/// QEMU's hub's descriptors: its device descriptor, its configuration with its one
/// interface and status endpoint, and its hub descriptor for eight ports, with no
/// power switching.
const HUB_DEVICE: [u8; 18] =
    [18, DEVICE, 0x10, 0x01, 9, 0, 0, 8, 0x09, 0x04, 0xAA, 0x55, 0x01, 0x01, 1, 2, 3, 1];
const HUB_CONFIGURATION: [u8; 25] = [
    9,
    CONFIGURATION,
    25,
    0,
    1,
    1,
    0,
    0xE0,
    0, //
    9,
    4,
    0,
    0,
    1,
    9,
    0,
    0,
    0, //
    7,
    5,
    0x81,
    3,
    2,
    0,
    0xFF,
];
const HUB_DESCRIPTOR: [u8; 10] = [10, HUB, PORTS as u8, 0x0A, 0x00, 0x01, 0x00, 0x00, 0x00, 0xFF];

/// QEMU's keyboard's descriptors: its device descriptor, its configuration with
/// its one boot keyboard interface, HID descriptor and report endpoint, and its
/// report descriptor: eight modifier bits, a reserved byte, five LEDs and six keys.
const KEYBOARD_DEVICE: [u8; 18] =
    [18, DEVICE, 0x00, 0x01, 0, 0, 0, 8, 0x27, 0x06, 0x01, 0x00, 0x00, 0x00, 1, 4, 11, 1];
const KEYBOARD_CONFIGURATION: [u8; 34] = [
    9,
    CONFIGURATION,
    34,
    0,
    1,
    1,
    8,
    0xA0,
    50, //
    9,
    4,
    0,
    0,
    1,
    3,
    1,
    1,
    0, //
    9,
    0x21,
    0x11,
    0x01,
    0,
    1,
    REPORT,
    63,
    0, //
    7,
    5,
    0x81,
    3,
    8,
    0,
    10,
];
const KEYBOARD_REPORT: [u8; 63] = [
    0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x75, 0x01, 0x95, 0x08, 0x05, 0x07, 0x19, 0xE0, 0x29, 0xE7,
    0x15, 0x00, 0x25, 0x01, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x95, 0x05, 0x75, 0x01,
    0x05, 0x08, 0x19, 0x01, 0x29, 0x05, 0x91, 0x02, 0x95, 0x01, 0x75, 0x03, 0x91, 0x01, 0x95, 0x06,
    0x75, 0x08, 0x15, 0x00, 0x25, 0xFF, 0x05, 0x07, 0x19, 0x00, 0x29, 0xFF, 0x81, 0x00, 0xC0,
];
/// The HID class requests NeOS uses: get and set the protocol and the idle rate,
/// and set a report (the LEDs). HID defines GET_REPORT too, which NeOS does not use.
const GET_REPORT: u8 = 1;
const GET_IDLE: u8 = 2;
const GET_PROTOCOL: u8 = 3;
const SET_REPORT: u8 = 9;
const SET_IDLE: u8 = 10;
const SET_PROTOCOL: u8 = 11;

/// A setup packet's fields.
#[derive(Clone, Copy, Default)]
struct Setup {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: usize,
}

impl Setup {
    fn parse(packet: &[u8]) -> Setup {
        let word = |at: usize| u16::from_le_bytes([packet[at], packet[at + 1]]);
        Setup {
            request_type: packet[0],
            request: packet[1],
            value: word(2),
            index: word(4),
            length: usize::from(word(6)),
        }
    }

    fn is_in(self) -> bool {
        self.request_type & IN != 0
    }
}

/// Where a control transfer is.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Stage {
    #[default]
    Idle,
    Data,
    Status,
}

/// A device's endpoint 0, which runs control transfers.
#[derive(Default)]
struct Control {
    stage: Stage,
    setup: Setup,
    /// The data, to send or as taken, and how far through it the transfer is.
    data: Vec<u8>,
    index: usize,
}

/// A port on the hub.
#[derive(Default)]
struct Port {
    status: u16,
    change: u16,
    device: Option<Box<Device>>,
}

impl Port {
    /// Sets status bits, and their change bits if they are the low five.
    fn set(&mut self, bits: u16) {
        if self.status & bits == 0 {
            self.status |= bits;
            self.change |= bits & 0x1F;
        }
    }

    /// Clears status bits, and sets their change bits if they are the low five.
    fn clear(&mut self, bits: u16) {
        if self.status & bits != 0 {
            self.status &= !bits;
            self.change |= bits & 0x1F;
        }
    }
}

/// What kind of device it is.
enum Kind {
    Hub { ports: [Port; PORTS] },
    Keyboard(Keyboard),
}

/// What a keyboard keeps.
#[derive(Default)]
struct Keyboard {
    /// 0 for the boot protocol, 1 for the report protocol, as HID numbers them.
    protocol: u8,
    /// The idle rate, in 4 ms steps, 0 for "only on a change".
    idle: u8,
    /// The LEDs the host last set.
    lights: u8,
    /// The modifier keys held, a bit each, as a report's first byte holds them.
    modifiers: u8,
    /// The other keys held, in the order they were pressed.
    held: Vec<u8>,
    /// The presses and releases still to come, from `--keys` (ADR 0025). A reset does
    /// not lose them, as the person typing would carry on.
    keys: VecDeque<Key>,
}

/// A key pressed or released, from `--keys`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key {
    /// Its usage on the keyboard page.
    pub usage: u8,
    /// Whether it is pressed, rather than released.
    pub pressed: bool,
}

/// The keys `--keys` knows, by the names QEMU's QMP gives them (its `QKeyCode`s),
/// with their usages on the keyboard page. Letters and digits are their own names.
const KEY_NAMES: [(&str, u8); 31] = [
    ("ret", 0x28),
    ("esc", 0x29),
    ("backspace", 0x2A),
    ("tab", 0x2B),
    ("spc", 0x2C),
    ("minus", 0x2D),
    ("equal", 0x2E),
    ("bracket_left", 0x2F),
    ("bracket_right", 0x30),
    ("backslash", 0x31),
    ("semicolon", 0x33),
    ("apostrophe", 0x34),
    ("grave_accent", 0x35),
    ("comma", 0x36),
    ("dot", 0x37),
    ("slash", 0x38),
    ("caps_lock", 0x39),
    ("home", 0x4A),
    ("delete", 0x4C),
    ("end", 0x4D),
    ("right", 0x4F),
    ("left", 0x50),
    ("down", 0x51),
    ("up", 0x52),
    ("ctrl", 0xE0),
    ("shift", 0xE1),
    ("alt", 0xE2),
    ("ctrl_r", 0xE4),
    ("shift_r", 0xE5),
    ("alt_r", 0xE6),
    ("num_lock", 0x53),
];

/// The keyboard page's first modifier key, Left Ctrl, and its last, Right GUI.
const FIRST_MODIFIER: u8 = 0xE0;
const LAST_MODIFIER: u8 = 0xE7;

/// Parses `--keys`'s list: names separated by commas, each after `+` for a press or
/// `-` for a release, such as `+shift,+h,-h,-shift`.
pub fn parse_keys(list: &str) -> Result<Vec<Key>, String> {
    list.split(',')
        .map(|item| {
            let (pressed, name) = match item.split_at_checked(1) {
                Some(("+", name)) => (true, name),
                Some(("-", name)) => (false, name),
                _ => return Err(format!("{item:?} does not start with + or -")),
            };
            let usage = match *name.as_bytes() {
                [letter @ b'a'..=b'z'] => 0x04 + (letter - b'a'),
                [b'0'] => 0x27,
                [digit @ b'1'..=b'9'] => 0x1E + (digit - b'1'),
                _ => match KEY_NAMES.iter().find(|&&(known, _)| known == name) {
                    Some(&(_, usage)) => usage,
                    None => return Err(format!("{name:?} is not a key Nebu knows")),
                },
            };
            Ok(Key { usage, pressed })
        })
        .collect()
}

impl Keyboard {
    /// Takes the next press or release, and returns the report that follows, as
    /// QEMU's keyboard handles one key a poll: the modifiers, a reserved byte, and
    /// the first six other keys held, or 1 in each if more are.
    fn next_report(&mut self) -> Option<[u8; 8]> {
        let key = self.keys.pop_front()?;
        if (FIRST_MODIFIER..=LAST_MODIFIER).contains(&key.usage) {
            let bit = 1 << (key.usage - FIRST_MODIFIER);
            if key.pressed {
                self.modifiers |= bit;
            } else {
                self.modifiers &= !bit;
            }
        } else if key.pressed {
            if !self.held.contains(&key.usage) {
                self.held.push(key.usage);
            }
        } else {
            self.held.retain(|&held| held != key.usage);
        }
        let mut report = [0; 8];
        report[0] = self.modifiers;
        if self.held.len() > 6 {
            report[2..].fill(1);
        } else {
            report[2..2 + self.held.len()].copy_from_slice(&self.held);
        }
        Some(report)
    }
}

/// A USB device.
pub struct Device {
    address: u8,
    configured: bool,
    control: Control,
    kind: Kind,
}

impl Device {
    /// QEMU's hub, with a keyboard on port 1, on which `keys` will be typed.
    pub fn hub_with_keyboard(keys: Vec<Key>) -> Device {
        let keyboard = Keyboard { keys: keys.into(), ..Keyboard::default() };
        let mut ports: [Port; PORTS] = Default::default();
        ports[0].device = Some(Box::new(Device::new(Kind::Keyboard(keyboard))));
        let mut hub = Device::new(Kind::Hub { ports });
        hub.reset();
        hub
    }

    fn new(kind: Kind) -> Device {
        let mut device =
            Device { address: 0, configured: false, control: Control::default(), kind };
        device.reset();
        device
    }

    /// Resets the device, as a port reset does: it forgets its address and
    /// configuration. A hub's ports go back to powered, with whatever is connected.
    pub fn reset(&mut self) {
        self.address = 0;
        self.configured = false;
        self.control = Control::default();
        match &mut self.kind {
            Kind::Hub { ports } => {
                for port in ports {
                    port.status = 0;
                    port.change = 0;
                    port.set(PORT_POWER);
                    if port.device.is_some() {
                        port.set(PORT_CONNECTION);
                    }
                }
            }
            Kind::Keyboard(keyboard) => {
                let keys = std::mem::take(&mut keyboard.keys);
                *keyboard = Keyboard { protocol: 1, keys, ..Keyboard::default() };
            }
        }
    }

    /// The device that answers to `address`: this one, or one behind an enabled
    /// port of this hub.
    pub fn find(&mut self, address: u8) -> Option<&mut Device> {
        if self.address == address {
            return Some(self);
        }
        let Kind::Hub { ports } = &mut self.kind else {
            return None;
        };
        ports
            .iter_mut()
            .filter(|port| port.status & PORT_ENABLE != 0)
            .find_map(|port| port.device.as_deref_mut()?.find(address))
    }

    /// Handles a packet to `endpoint`: `data` for a setup or OUT packet, or room
    /// for `room` bytes for an IN one.
    pub fn handle(&mut self, endpoint: u8, token: Token, data: &[u8], room: usize) -> Answer {
        if endpoint == 0 {
            return self.control(token, data, room);
        }
        if endpoint != 1 || token != Token::In {
            return Err("a USB transfer to an endpoint the device does not have");
        }
        if !self.configured {
            return Err("a USB interrupt transfer to a device not yet configured");
        }
        match &mut self.kind {
            Kind::Hub { ports } => {
                // A bit for each port with a change, from bit 1, in two bytes.
                let changed = ports
                    .iter()
                    .enumerate()
                    .filter(|(_, port)| port.change != 0)
                    .fold(0u16, |bits, (index, _)| bits | 1 << (index + 1));
                let length = if room == 1 { 1 } else { 2 };
                if room < length {
                    return Err("a hub status transfer too short for its two bytes");
                }
                if changed == 0 {
                    return Ok(Reply::Nak);
                }
                Ok(Reply::sent(changed.to_le_bytes()[..length].to_vec()))
            }
            Kind::Keyboard(keyboard) => match keyboard.next_report() {
                Some(report) => Ok(Reply::sent(report[..room.min(8)].to_vec())),
                None => Ok(Reply::Nak),
            },
        }
    }

    /// Runs endpoint 0's part of a control transfer.
    fn control(&mut self, token: Token, data: &[u8], room: usize) -> Answer {
        let setup = self.control.setup;
        match (token, self.control.stage) {
            (Token::Setup, _) => {
                if data.len() != 8 {
                    return Ok(Reply::Stall);
                }
                let setup = Setup::parse(data);
                if setup.length > LARGEST_CONTROL {
                    return Ok(Reply::Stall);
                }
                self.control = Control { stage: Stage::Idle, setup, data: Vec::new(), index: 0 };
                if setup.is_in() {
                    // A request for data is answered at once, and its data sent after.
                    let Some(mut answer) = self.request(setup, &[])? else {
                        return Ok(Reply::Stall);
                    };
                    answer.truncate(setup.length);
                    self.control.data = answer;
                    self.control.stage = Stage::Data;
                } else {
                    self.control.stage =
                        if setup.length == 0 { Stage::Status } else { Stage::Data };
                }
                Ok(Reply::took(8))
            }
            (Token::In, Stage::Status) if !setup.is_in() => {
                // The status stage of a request with no data for the host: now it
                // takes effect.
                self.control.stage = Stage::Idle;
                let data = std::mem::take(&mut self.control.data);
                match self.request(setup, &data)? {
                    Some(_) => Ok(Reply::took(0)),
                    None => Ok(Reply::Stall),
                }
            }
            (Token::In, Stage::Status) => Ok(Reply::took(0)),
            (Token::In, Stage::Data) if setup.is_in() => {
                let control = &mut self.control;
                let end = control.data.len().min(control.index + room);
                let part = control.data[control.index..end].to_vec();
                control.index = end;
                if control.index >= control.data.len() {
                    control.stage = Stage::Status;
                }
                Ok(Reply::sent(part))
            }
            (Token::Out, Stage::Status) => {
                if setup.is_in() {
                    self.control.stage = Stage::Idle;
                }
                Ok(Reply::took(0))
            }
            (Token::Out, Stage::Data) if !setup.is_in() => {
                let control = &mut self.control;
                let part = data.len().min(setup.length - control.index);
                control.data.extend_from_slice(&data[..part]);
                control.index += part;
                if control.index >= setup.length {
                    control.stage = Stage::Status;
                }
                Ok(Reply::took(part))
            }
            (_, Stage::Data) => {
                self.control.stage = Stage::Idle;
                Ok(Reply::Stall)
            }
            (_, Stage::Idle) => Ok(Reply::Stall),
        }
    }

    /// Carries out a request, with the data the host sent for it. Returns the data
    /// for the host, or `None` for a stall.
    fn request(&mut self, setup: Setup, data: &[u8]) -> Result<Option<Vec<u8>>, &'static str> {
        let [kind, number] = setup.value.to_be_bytes();
        match (setup.request_type, setup.request) {
            (0x80, GET_DESCRIPTOR) => {
                let descriptor: &[u8] = match (kind, number, &self.kind) {
                    (DEVICE, 0, Kind::Hub { .. }) => &HUB_DEVICE,
                    (DEVICE, 0, Kind::Keyboard(_)) => &KEYBOARD_DEVICE,
                    (CONFIGURATION, 0, Kind::Hub { .. }) => &HUB_CONFIGURATION,
                    (CONFIGURATION, 0, Kind::Keyboard(_)) => &KEYBOARD_CONFIGURATION,
                    _ => return Err("a USB descriptor that Nebu does not model"),
                };
                Ok(Some(descriptor.to_vec()))
            }
            (0x00, SET_ADDRESS) if setup.value < 128 => {
                self.address = setup.value as u8;
                Ok(Some(Vec::new()))
            }
            (0x00, SET_CONFIGURATION) if setup.value <= 1 => {
                self.configured = setup.value == 1;
                Ok(Some(Vec::new()))
            }
            _ => match &mut self.kind {
                Kind::Hub { ports } => hub_request(ports, setup),
                Kind::Keyboard(keyboard) => keyboard_request(keyboard, setup, data),
            },
        }
    }
}

/// Carries out a hub class request.
fn hub_request(ports: &mut [Port; PORTS], setup: Setup) -> Result<Option<Vec<u8>>, &'static str> {
    let [kind, _] = setup.value.to_be_bytes();
    if (setup.request_type, setup.request) == (0xA0, GET_DESCRIPTOR) && kind == HUB {
        return Ok(Some(HUB_DESCRIPTOR.to_vec()));
    }
    let port = usize::from(setup.index)
        .checked_sub(1)
        .and_then(|index| ports.get_mut(index))
        .ok_or("a USB hub request for a port the hub does not have")?;
    match (setup.request_type, setup.request, setup.value) {
        (0xA3, GET_STATUS, 0) => {
            let [status_low, status_high] = port.status.to_le_bytes();
            let [change_low, change_high] = port.change.to_le_bytes();
            Ok(Some(vec![status_low, status_high, change_low, change_high]))
        }
        (0x23, SET_FEATURE, FEATURE_RESET) => {
            port.set(PORT_RESET);
            port.clear(PORT_RESET);
            if let Some(device) = port.device.as_deref_mut() {
                device.reset();
                port.set(PORT_ENABLE);
            }
            Ok(Some(Vec::new()))
        }
        // QEMU's hub has no power switching: its ports are always powered.
        (0x23, SET_FEATURE, FEATURE_POWER) => Ok(Some(Vec::new())),
        (0x23, CLEAR_FEATURE, FEATURE_ENABLE) => {
            port.status &= !PORT_ENABLE;
            Ok(Some(Vec::new()))
        }
        (0x23, CLEAR_FEATURE, feature) if FEATURE_CHANGES.contains(&feature) => {
            port.change &= !(1 << (feature - 16));
            Ok(Some(Vec::new()))
        }
        _ => Err("a USB hub request that Nebu does not model"),
    }
}

/// Carries out a request to the keyboard's interface.
fn keyboard_request(
    keyboard: &mut Keyboard,
    setup: Setup,
    data: &[u8],
) -> Result<Option<Vec<u8>>, &'static str> {
    let [kind, _] = setup.value.to_be_bytes();
    match (setup.request_type, setup.request) {
        (0x81, GET_DESCRIPTOR) if kind == REPORT => Ok(Some(KEYBOARD_REPORT.to_vec())),
        (0xA1, GET_PROTOCOL) => Ok(Some(vec![keyboard.protocol])),
        (0x21, SET_PROTOCOL) if setup.value <= 1 => {
            keyboard.protocol = setup.value as u8;
            Ok(Some(Vec::new()))
        }
        (0xA1, GET_IDLE) => Ok(Some(vec![keyboard.idle])),
        (0x21, SET_IDLE) => {
            keyboard.idle = kind;
            Ok(Some(Vec::new()))
        }
        (0x21, SET_REPORT) => {
            if let Some(&lights) = data.first() {
                keyboard.lights = lights;
            }
            Ok(Some(Vec::new()))
        }
        (0x21 | 0xA1, GET_REPORT) => Err("a HID Get Report, which Nebu does not model"),
        (0x21 | 0xA1, _) => Ok(None),
        _ => Err("a USB keyboard request that Nebu does not model"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> Vec<u8> {
        let mut packet = vec![request_type, request];
        for word in [value, index, length] {
            packet.extend_from_slice(&word.to_le_bytes());
        }
        packet
    }

    /// Runs a whole control transfer reading up to `length` bytes, in 8-byte
    /// packets, and returns them, or `None` for a stall.
    fn read(
        device: &mut Device,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Option<Vec<u8>> {
        let packet = setup(request_type, request, value, index, length);
        if device.handle(0, Token::Setup, &packet, 8).unwrap() == Reply::Stall {
            return None;
        }
        let mut data = Vec::new();
        loop {
            match device.handle(0, Token::In, &[], 8).unwrap() {
                Reply::Ack { data: part, .. } => {
                    let short = part.len() < 8;
                    data.extend(part);
                    if short {
                        break;
                    }
                }
                _ => return None,
            }
        }
        assert_eq!(device.handle(0, Token::Out, &[], 0).unwrap(), Reply::took(0));
        Some(data)
    }

    fn write(device: &mut Device, request_type: u8, request: u8, value: u16, index: u16) -> Reply {
        let packet = setup(request_type, request, value, index, 0);
        assert_eq!(device.handle(0, Token::Setup, &packet, 8).unwrap(), Reply::took(8));
        device.handle(0, Token::In, &[], 0).unwrap()
    }

    #[test]
    fn the_hub_takes_its_address_at_the_status_stage() {
        let mut hub = Device::hub_with_keyboard(Vec::new());
        let descriptor = read(&mut hub, 0x80, GET_DESCRIPTOR, 0x0100, 0, 18).unwrap();
        assert_eq!(descriptor, HUB_DEVICE);
        let packet = setup(0x00, SET_ADDRESS, 1, 0, 0);
        hub.handle(0, Token::Setup, &packet, 8).unwrap();
        assert!(hub.find(0).is_some());
        assert_eq!(hub.handle(0, Token::In, &[], 0).unwrap(), Reply::took(0));
        assert!(hub.find(0).is_none());
        assert!(hub.find(1).is_some());
    }

    #[test]
    fn a_reset_port_reaches_the_keyboard() {
        let mut hub = Device::hub_with_keyboard(Vec::new());
        hub.address = 1;
        assert!(hub.find(0).is_none());
        let status = read(&mut hub, 0xA3, GET_STATUS, 0, 1, 4).unwrap();
        assert_eq!(status, [0x01, 0x01, 0x01, 0x00]);
        assert_eq!(write(&mut hub, 0x23, SET_FEATURE, FEATURE_RESET, 1), Reply::took(0));
        let status = read(&mut hub, 0xA3, GET_STATUS, 0, 1, 4).unwrap();
        assert_eq!(status, [0x03, 0x01, 0x13, 0x00]);
        let keyboard = hub.find(0).unwrap();
        let report = read(keyboard, 0x81, GET_DESCRIPTOR, 0x2200, 0, 256).unwrap();
        assert_eq!(report.len(), 63);
    }

    #[test]
    fn the_hub_reports_changed_ports_once_configured() {
        let mut hub = Device::hub_with_keyboard(Vec::new());
        assert!(hub.handle(1, Token::In, &[], 2).is_err());
        write(&mut hub, 0x00, SET_CONFIGURATION, 1, 0);
        assert_eq!(hub.handle(1, Token::In, &[], 2).unwrap(), Reply::sent(vec![0b10, 0]));
        write(&mut hub, 0x23, CLEAR_FEATURE, 16, 1);
        assert_eq!(hub.handle(1, Token::In, &[], 2).unwrap(), Reply::Nak);
    }

    #[test]
    fn keys_parse_by_their_qemu_names() {
        let keys = parse_keys("+shift,+h,-h,-shift,+0,+caps_lock").unwrap();
        let usages: Vec<(u8, bool)> = keys.iter().map(|key| (key.usage, key.pressed)).collect();
        assert_eq!(
            usages,
            [(0xE1, true), (0x0B, true), (0x0B, false), (0xE1, false), (0x27, true), (0x39, true)]
        );
        assert!(parse_keys("shift").is_err());
        assert!(parse_keys("+hyper").is_err());
        assert!(parse_keys("").is_err());
    }

    #[test]
    fn the_keyboard_reports_one_key_a_poll() {
        let keys = parse_keys("+shift,+h,+i,-h,-shift,-i").unwrap();
        let mut hub = Device::hub_with_keyboard(keys);
        write(&mut hub, 0x00, SET_ADDRESS, 1, 0);
        write(&mut hub, 0x23, SET_FEATURE, 4, 1);
        let keyboard = hub.find(0).unwrap();
        write(keyboard, 0x00, SET_CONFIGURATION, 1, 0);
        let reports: Vec<Reply> =
            (0..7).map(|_| keyboard.handle(1, Token::In, &[], 8).unwrap()).collect();
        let sent = |report: [u8; 8]| Reply::sent(report.to_vec());
        assert_eq!(
            reports,
            [
                sent([2, 0, 0, 0, 0, 0, 0, 0]),
                sent([2, 0, 0x0B, 0, 0, 0, 0, 0]),
                sent([2, 0, 0x0B, 0x0C, 0, 0, 0, 0]),
                sent([2, 0, 0x0C, 0, 0, 0, 0, 0]),
                sent([0, 0, 0x0C, 0, 0, 0, 0, 0]),
                sent([0; 8]),
                Reply::Nak,
            ]
        );
    }

    #[test]
    fn the_keyboard_stalls_requests_hid_does_not_define() {
        let mut keyboard = Device::new(Kind::Keyboard(Keyboard::default()));
        assert_eq!(read(&mut keyboard, 0xA1, 7, 0, 0, 8), None);
        assert_eq!(read(&mut keyboard, 0xA1, GET_PROTOCOL, 0, 0, 1), Some(vec![1]));
        assert!(
            keyboard
                .request(
                    Setup {
                        request_type: 0x80,
                        request: GET_DESCRIPTOR,
                        value: 0x0300,
                        ..Setup::default()
                    },
                    &[]
                )
                .is_err()
        );
    }
}
