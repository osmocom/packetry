use std::ops::Range;

use crate::file_vec::FileVec;
use crate::hybrid_index::HybridIndex;
use bytemuck_derive::{Pod, Zeroable};
use num_enum::{IntoPrimitive, FromPrimitive};
use num_format::{Locale, ToFormattedString};
use humansize::{FileSize, file_size_opts as options};

#[derive(Copy, Clone, Debug, IntoPrimitive, FromPrimitive, PartialEq)]
#[repr(u8)]
enum PID {
    RSVD  = 0xF0,
    OUT   = 0xE1,
    ACK   = 0xD2,
    DATA0 = 0xC3,
    PING  = 0xB4,
    SOF   = 0xA5,
    NYET  = 0x96,
    DATA2 = 0x87,
    SPLIT = 0x78,
    IN    = 0x69,
    NAK   = 0x5A,
    DATA1 = 0x4B,
    ERR   = 0x3C,
    SETUP = 0x2D,
    STALL = 0x1E,
    MDATA = 0x0F,
    #[default]
    Malformed = 0,
}

impl Default for PID {
    fn default() -> Self {
        PID::Malformed
    }
}

impl std::fmt::Display for PID {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone)]
pub enum Item {
    Transfer(u64),
    Transaction(u64, u64),
    Packet(u64, u64, u64),
}

bitfield! {
    #[derive(Debug)]
    pub struct SOFFields(u16);
    u16, frame_number, _: 10, 0;
    u8, crc, _: 15, 11;
}

bitfield! {
    #[derive(Debug)]
    pub struct TokenFields(u16);
    u8, device_address, _: 6, 0;
    u8, endpoint_number, _: 10, 7;
    u8, crc, _: 15, 11;
}

#[derive(Debug)]
pub struct DataFields {
    pub crc: u16,
}

#[derive(Debug)]
pub enum PacketFields {
    SOF(SOFFields),
    Token(TokenFields),
    Data(DataFields),
    None
}

impl PacketFields {
    fn from_packet(packet: &[u8]) -> Self {
        let end = packet.len();
        use PID::*;
        match PID::from(packet[0]) {
            SOF => PacketFields::SOF(
                SOFFields(
                    u16::from_le_bytes([packet[1], packet[2]]))),
            SETUP | IN | OUT => PacketFields::Token(
                TokenFields(
                    u16::from_le_bytes([packet[1], packet[2]]))),
            DATA0 | DATA1 => PacketFields::Data(
                DataFields{
                    crc: u16::from_le_bytes(
                        [packet[end - 2], packet[end - 1]])}),
            _ => PacketFields::None
        }
    }
}

#[derive(Copy, Clone, Debug, FromPrimitive)]
#[repr(u8)]
pub enum RequestType {
    Standard = 0,
    Class = 1,
    Vendor = 2,
    #[default]
    Reserved = 3,
}

#[derive(Copy, Clone, Debug, FromPrimitive)]
#[repr(u8)]
pub enum Recipient {
    Device = 0,
    Interface = 1,
    Endpoint = 2,
    Other = 3,
    #[default]
    Reserved = 4,
}

#[derive(Copy, Clone, Debug, FromPrimitive)]
#[repr(u8)]
pub enum Direction {
    #[default]
    Out = 0,
    In = 1,
}

bitfield! {
    #[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
    #[repr(C)]
    pub struct RequestTypeFields(u8);
    u8, _recipient, _: 4, 0;
    u8, _type, _: 6, 5;
    u8, _direction, _: 7, 7;
}

impl RequestTypeFields {
    pub fn recipient(&self) -> Recipient { Recipient::from(self._recipient()) }
    pub fn request_type(&self) -> RequestType { RequestType::from(self._type()) }
    pub fn direction(&self) -> Direction { Direction::from(self._direction()) }
}

pub struct SetupFields {
    type_fields: RequestTypeFields,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
}

impl SetupFields {
    fn from_data_packet(packet: &[u8]) -> Self {
        SetupFields {
            type_fields: RequestTypeFields(packet[1]),
            request: packet[2],
            value: u16::from_le_bytes([packet[3], packet[4]]),
            index: u16::from_le_bytes([packet[5], packet[6]]),
            length: u16::from_le_bytes([packet[7], packet[8]]),
        }
    }
}

#[derive(Debug, FromPrimitive)]
#[repr(u8)]
pub enum StandardRequest {
    GetStatus = 0,
    ClearFeature = 1,
    SetFeature = 3,
    SetAddress = 5,
    GetDescriptor = 6,
    SetDescriptor = 7,
    GetConfiguration = 8,
    SetConfiguration = 9,
    GetInterface = 10,
    SetInterface = 11,
    SynchFrame = 12,
    #[default]
    Unknown = 13,
}

impl StandardRequest {
    pub fn description(&self, fields: &SetupFields) -> String {
        use StandardRequest::*;
        match self {
            GetStatus => format!("Getting status"),
            ClearFeature | SetFeature => {
                let feature = StandardFeature::from(fields.value);
                format!("{} {}",
                    match self {
                        ClearFeature => "Clearing",
                        SetFeature => "Setting",
                        _ => ""
                    },
                    feature.description()
                )
            },
            SetAddress => format!("Setting address to {}", fields.value),
            GetDescriptor | SetDescriptor => {
                let descriptor_type =
                    DescriptorType::from((fields.value >> 8) as u8);
                format!(
                    "{} {} descriptor #{}{}",
                    match self {
                        GetDescriptor => "Getting",
                        SetDescriptor => "Setting",
                        _ => ""
                    },
                    descriptor_type.description(),
                    fields.value & 0xFF,
                    match (descriptor_type, fields.index) {
                        (DescriptorType::String, language) if language > 0 =>
                            format!(", language 0x{:04x}", language),
                        (..) => format!(""),
                    }
                )
            },
            GetConfiguration => format!("Getting configuration"),
            SetConfiguration => format!("Setting configuration {}", fields.value),
            GetInterface => format!("Getting interface {}", fields.index),
            SetInterface => format!("Setting interface {} to {}",
                                    fields.index, fields.value),
            SynchFrame => format!("Synchronising frame"),
            Unknown => format!("Unknown standard request"),
        }
    }
}

#[derive(Copy, Clone, Debug, FromPrimitive)]
#[repr(u8)]
pub enum DescriptorType {
    Device = 1,
    Configuration = 2,
    String = 3,
    Interface = 4,
    Endpoint = 5,
    DeviceQualifier = 6,
    OtherSpeedConfiguration = 7,
    InterfacePower = 8,
    #[default]
    Unknown = 9
}

impl DescriptorType {
    pub fn description(self) -> &'static str {
        const STRINGS: [&str; 10] = [
            "invalid",
            "device",
            "configuration",
            "string",
            "interface",
            "endpoint",
            "device qualifier",
            "other speed configuration",
            "interface power",
            "unknown",
        ];
        STRINGS[self as usize]
    }
}

#[derive(Copy, Clone, Debug, FromPrimitive)]
#[repr(u16)]
pub enum StandardFeature {
    EndpointHalt = 0,
    DeviceRemoteWakeup = 1,
    TestMode = 2,
    #[default]
    Unknown = 3
}

impl StandardFeature {
    pub fn description(self) -> &'static str {
        const STRINGS: [&str; 4] = [
            "endpoint halt",
            "device remote wakeup",
            "test mode",
            "unknown standard feature",
        ];
        STRINGS[self as usize]
    }
}

#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Endpoint {
    pub device_address: u8,
    pub endpoint_number: u8,
}

bitfield! {
    #[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
    #[repr(C)]
    pub struct TransferIndexEntry(u64);
    u64, transfer_id, set_transfer_id: 51, 0;
    u16, endpoint_id, set_endpoint_id: 62, 52;
    u8, _is_start, _set_is_start: 63, 63;
}

impl TransferIndexEntry {
    fn is_start(&self) -> bool {
        self._is_start() != 0
    }
    fn set_is_start(&mut self, value: bool) {
        self._set_is_start(value as u8)
    }
}

#[derive(Copy, Clone, Debug, Default)]
struct TransactionState {
    first: PID,
    last: PID,
    start: u64,
    count: u64,
    endpoint_id: usize,
}

#[derive(Copy, Clone, IntoPrimitive, FromPrimitive, PartialEq)]
#[repr(u8)]
enum EndpointState {
    #[default]
    Idle = 0,
    Starting = 1,
    Ongoing = 2,
    Ending = 3,
}

#[derive(FromPrimitive)]
#[repr(u8)]
enum EndpointType {
    Control = 0,
    #[default]
    Normal,
    Framing = 0xFE,
    Invalid = 0xFF,
}

struct EndpointData {
    ep_type: EndpointType,
    transaction_ids: HybridIndex,
    transfer_index: HybridIndex,
    transaction_start: u64,
    transaction_count: u64,
    last: PID,
}

impl EndpointData {
    fn status(&self, next: PID) -> DecodeStatus {
        use PID::*;
        use EndpointType::*;
        match (&self.ep_type, self.last, next) {

            // A SETUP transaction starts a new control transfer.
            (Control, _, SETUP) => DecodeStatus::NEW,

            // SETUP may be followed by IN or OUT at data stage.
            (Control, SETUP, IN | OUT) => DecodeStatus::CONTINUE,

            // IN or OUT may then be repeated during data stage.
            (Control, IN, IN) => DecodeStatus::CONTINUE,
            (Control, OUT, OUT) => DecodeStatus::CONTINUE,

            // The opposite direction at status stage ends the transfer.
            (Control, IN, OUT) => DecodeStatus::DONE,
            (Control, OUT, IN) => DecodeStatus::DONE,

            // An IN or OUT transaction on a non-control endpoint,
            // with no transfer in progress, starts a bulk transfer.
            (Normal, Malformed, IN | OUT) => DecodeStatus::NEW,

            // IN or OUT may then be repeated.
            (Normal, IN, IN) => DecodeStatus::CONTINUE,
            (Normal, OUT, OUT) => DecodeStatus::CONTINUE,

            // A SOF group starts a special transfer, unless
            // one is already in progress.
            (Framing, Malformed, SOF) => DecodeStatus::NEW,

            // Further SOF groups continue this transfer.
            (Framing, SOF, SOF) => DecodeStatus::CONTINUE,

            // Any other case is not a valid part of a transfer.
            _ => DecodeStatus::INVALID
        }
    }
}

const USB_MAX_DEVICES: usize = 128;
const USB_MAX_ENDPOINTS: usize = 16;

pub struct Capture {
    item_index: HybridIndex,
    packet_index: HybridIndex,
    packet_data: FileVec<u8>,
    transaction_index: HybridIndex,
    transfer_index: FileVec<TransferIndexEntry>,
    endpoint_index: [[i16; USB_MAX_ENDPOINTS]; USB_MAX_DEVICES],
    endpoints: FileVec<Endpoint>,
    endpoint_data: Vec<EndpointData>,
    endpoint_states: FileVec<u8>,
    endpoint_state_index: HybridIndex,
    last_endpoint_state: Vec<u8>,
    last_item_endpoint: i16,
    transaction_state: TransactionState,
}

impl Default for Capture {
    fn default() -> Self {
        Capture::new()
    }
}

#[derive(PartialEq)]
enum DecodeStatus {
    NEW,
    CONTINUE,
    DONE,
    INVALID
}

impl TransactionState {
    pub fn status(&self, next: PID) -> DecodeStatus {
        use PID::*;
        match (self.first, self.last, next) {

            // SETUP, IN or OUT always start a new transaction.
            (_, _, SETUP | IN | OUT) => DecodeStatus::NEW,

            // SOF when there is no existing transaction starts a new
            // "transaction" representing an idle period on the bus.
            (_, Malformed, SOF) => DecodeStatus::NEW,
            // Additional SOFs extend this "transaction", more may follow.
            (_, SOF, SOF) => DecodeStatus::CONTINUE,

            // SETUP must be followed by DATA0, wait for ACK to follow.
            (_, SETUP, DATA0) => DecodeStatus::CONTINUE,
            // ACK then completes the transaction.
            (SETUP, DATA0, ACK) => DecodeStatus::DONE,

            // IN may be followed by NAK or STALL, completing transaction.
            (_, IN, NAK | STALL) => DecodeStatus::DONE,
            // IN or OUT may be followed by DATA0 or DATA1, wait for status.
            (_, IN | OUT, DATA0 | DATA1) => DecodeStatus::CONTINUE,
            // An ACK then completes the transaction.
            (IN | OUT, DATA0 | DATA1, ACK) => DecodeStatus::DONE,
            // OUT may also be completed by NAK or STALL.
            (OUT, DATA0 | DATA1, NAK | STALL) => DecodeStatus::DONE,

            // Any other case is not a valid part of a transaction.
            _ => DecodeStatus::INVALID,
        }
    }
}

fn get_index_range(index: &mut HybridIndex,
                      length: u64,
                      id: u64) -> Range<u64>
{
    if id + 2 > index.len() {
        let start = index.get(id).unwrap();
        let end = length;
        start..end
    } else {
        let vec = index.get_range(id..(id + 2)).unwrap();
        let start = vec[0];
        let end = vec[1];
        start..end
    }
}

pub fn fmt_count(count: u64) -> String {
    count.to_formatted_string(&Locale::en)
}

pub fn fmt_size(size: u64) -> String {
    size.file_size(options::BINARY).unwrap()
}

pub fn fmt_vec<T>(vec: &FileVec<T>) -> String
    where T: bytemuck::Pod + Default
{
    format!("{} entries, {}", fmt_count(vec.len()), fmt_size(vec.size()))
}

pub fn fmt_index(idx: &HybridIndex) -> String {
    format!("{} values in {} entries, {}",
            fmt_count(idx.len()),
            fmt_count(idx.entry_count()),
            fmt_size(idx.size()))
}

impl Capture {
    pub fn new() -> Self {
        let mut capture = Capture {
            item_index: HybridIndex::new(1).unwrap(),
            packet_index: HybridIndex::new(2).unwrap(),
            packet_data: FileVec::new().unwrap(),
            transaction_index: HybridIndex::new(1).unwrap(),
            transfer_index: FileVec::new().unwrap(),
            endpoints: FileVec::new().unwrap(),
            endpoint_data: Vec::new(),
            endpoint_index: [[-1; USB_MAX_ENDPOINTS]; USB_MAX_DEVICES],
            endpoint_states: FileVec::new().unwrap(),
            endpoint_state_index: HybridIndex::new(1).unwrap(),
            last_endpoint_state: Vec::new(),
            last_item_endpoint: -1,
            transaction_state: TransactionState::default(),
        };
        capture.add_endpoint(0, EndpointType::Invalid as usize);
        capture.add_endpoint(0, EndpointType::Framing as usize);
        capture
    }

    pub fn handle_raw_packet(&mut self, packet: &[u8]) {
        self.transaction_update(packet);
        self.packet_index.push(self.packet_data.len()).unwrap();
        self.packet_data.append(packet).unwrap();
    }

    pub fn print_storage_summary(&self) {
        let mut overhead: u64 =
            self.packet_index.size() +
            self.transaction_index.size() +
            self.transfer_index.size() +
            self.endpoint_states.size() +
            self.endpoint_state_index.size();
        let mut trx_count = 0;
        let mut trx_entries = 0;
        let mut trx_size = 0;
        let mut xfr_count = 0;
        let mut xfr_entries = 0;
        let mut xfr_size = 0;
        for ep_data in &self.endpoint_data {
            trx_count += ep_data.transaction_ids.len();
            trx_entries += ep_data.transaction_ids.entry_count();
            trx_size += ep_data.transaction_ids.size();
            xfr_count += ep_data.transfer_index.len();
            xfr_entries += ep_data.transfer_index.entry_count();
            xfr_size += ep_data.transfer_index.size();
            overhead += trx_size + xfr_size;
        }
        let ratio = (overhead as f32) / (self.packet_data.size() as f32);
        let percentage = ratio * 100.0;
        print!(concat!(
            "Storage summary:\n",
            "  Packet data: {}\n",
            "  Packet index: {}\n",
            "  Transaction index: {}\n",
            "  Transfer index: {}\n",
            "  Endpoint states: {}\n",
            "  Endpoint state index: {}\n",
            "  Endpoint transaction indices: {} values in {} entries, {}\n",
            "  Endpoint transfer indices: {} values in {} entries, {}\n",
            "Total overhead: {:.1}% ({})\n"),
            fmt_size(self.packet_data.size()),
            fmt_index(&self.packet_index),
            fmt_index(&self.transaction_index),
            fmt_vec(&self.transfer_index),
            fmt_vec(&self.endpoint_states),
            fmt_index(&self.endpoint_state_index),
            fmt_count(trx_count), fmt_count(trx_entries), fmt_size(trx_size),
            fmt_count(xfr_count), fmt_count(xfr_entries), fmt_size(xfr_size),
            percentage, fmt_size(overhead),
        )
    }

    fn transaction_update(&mut self, packet: &[u8]) {
        let pid = PID::from(packet[0]);
        match self.transaction_state.status(pid) {
            DecodeStatus::NEW => {
                self.transaction_end();
                self.transaction_start(packet);
            },
            DecodeStatus::CONTINUE => {
                self.transaction_append(pid);
            },
            DecodeStatus::DONE => {
                self.transaction_append(pid);
                self.transaction_end();
            },
            DecodeStatus::INVALID => {
                self.transaction_end();
                self.transaction_start(packet);
                self.transaction_end();
            },
        };
    }

    fn transaction_start(&mut self, packet: &[u8]) {
        let state = &mut self.transaction_state;
        state.start = self.packet_index.len();
        state.count = 1;
        state.first = PID::from(packet[0]);
        state.last = state.first;
        match PacketFields::from_packet(&packet) {
            PacketFields::SOF(_) => {
                self.transaction_state.endpoint_id = 1;
            },
            PacketFields::Token(token) => {
                let addr = token.device_address() as usize;
                let num = token.endpoint_number() as usize;
                if self.endpoint_index[addr][num] < 0 {
                    let endpoint_id = self.endpoints.len() as i16;
                    self.endpoint_index[addr][num] = endpoint_id;
                    self.add_endpoint(addr, num);
                }
                self.transaction_state.endpoint_id =
                    self.endpoint_index[addr][num] as usize;
            },
            _ => {
                self.transaction_state.endpoint_id = 0;
            }
        }
    }

    fn transaction_append(&mut self, pid: PID) {
        let state = &mut self.transaction_state;
        state.count += 1;
        state.last = pid;
    }

    fn transaction_end(&mut self) {
        self.add_transaction();
        let state = &mut self.transaction_state;
        state.count = 0;
        state.first = PID::Malformed;
        state.last = PID::Malformed;
    }

    fn add_transaction(&mut self) {
        if self.transaction_state.count == 0 { return }
        self.transfer_update();
        self.transaction_index.push(self.transaction_state.start).unwrap();
    }

    fn add_endpoint(&mut self, addr: usize, num: usize) {
        let ep_data = EndpointData {
            ep_type: EndpointType::from(num as u8),
            transaction_ids: HybridIndex::new(1).unwrap(),
            transfer_index: HybridIndex::new(1).unwrap(),
            transaction_start: 0,
            transaction_count: 0,
            last: PID::Malformed,
        };
        self.endpoint_data.push(ep_data);
        let endpoint = Endpoint {
            device_address: addr as u8,
            endpoint_number: num as u8,
        };
        self.endpoints.push(&endpoint).unwrap();
        self.last_endpoint_state.push(EndpointState::Idle as u8);
    }

    fn transfer_update(&mut self) {
        let endpoint_id = self.transaction_state.endpoint_id;
        let ep_data = &mut self.endpoint_data[endpoint_id];
        let status = ep_data.status(self.transaction_state.first);
        let completed =
            self.transaction_state.count == 3 &&
            self.transaction_state.last == PID::ACK;
        let retry_needed =
            ep_data.transaction_count > 0 &&
            status != DecodeStatus::INVALID &&
            !completed;
        if retry_needed {
            self.transfer_append(false);
            return
        }
        match status {
            DecodeStatus::NEW => {
                self.transfer_end();
                self.transfer_start();
                self.transfer_append(true);
            },
            DecodeStatus::CONTINUE => {
                self.transfer_append(true);
            },
            DecodeStatus::DONE => {
                self.transfer_append(true);
                self.transfer_end();
            },
            DecodeStatus::INVALID => {
                self.transfer_end();
                self.transfer_start();
                self.transfer_append(false);
                self.transfer_end();
            }
        }
    }

    fn transfer_start(&mut self) {
        self.item_index.push(self.transfer_index.len()).unwrap();
        let endpoint_id = self.transaction_state.endpoint_id;
        self.last_item_endpoint = endpoint_id as i16;
        self.add_transfer_entry(endpoint_id, true);
        let ep_data = &mut self.endpoint_data[endpoint_id];
        ep_data.transaction_start = ep_data.transaction_ids.len();
        ep_data.transaction_count = 0;
        ep_data.transfer_index.push(ep_data.transaction_start).unwrap();
    }

    fn transfer_append(&mut self, success: bool) {
        let endpoint_id = self.transaction_state.endpoint_id;
        let ep_data = &mut self.endpoint_data[endpoint_id];
        ep_data.transaction_ids.push(self.transaction_index.len()).unwrap();
        ep_data.transaction_count += 1;
        if success {
            ep_data.last = self.transaction_state.first;
        }
    }

    fn transfer_end(&mut self) {
        let endpoint_id = self.transaction_state.endpoint_id;
        let ep_data = &self.endpoint_data[endpoint_id];
        if ep_data.transaction_count > 0 {
            if self.last_item_endpoint != (endpoint_id as i16) {
                self.item_index.push(self.transfer_index.len()).unwrap();
                self.last_item_endpoint = endpoint_id as i16;
            }
            self.add_transfer_entry(endpoint_id, false);
        }
        let ep_data = &mut self.endpoint_data[endpoint_id];
        ep_data.transaction_count = 0;
        ep_data.last = PID::Malformed;
    }

    fn add_transfer_entry(&mut self, endpoint_id: usize, start: bool) {
        let ep_data = &mut self.endpoint_data[endpoint_id];
        let mut entry = TransferIndexEntry::default();
        entry.set_endpoint_id(endpoint_id as u16);
        entry.set_transfer_id(ep_data.transfer_index.len());
        entry.set_is_start(start);
        self.transfer_index.push(&entry).unwrap();
        self.add_endpoint_state(endpoint_id, start);
    }

    fn add_endpoint_state(&mut self, endpoint_id: usize, start: bool) {
        let endpoint_count = self.endpoints.len() as usize;
        for i in 0..endpoint_count {
            use EndpointState::*;
            self.last_endpoint_state[i] = {
                let same = endpoint_id == i;
                let last = EndpointState::from(self.last_endpoint_state[i]);
                match (same, start, last) {
                    (true, true,  _)               => Starting,
                    (true, false, _)               => Ending,
                    (false, _, Starting | Ongoing) => Ongoing,
                    (false, _, Ending | Idle)      => Idle,
                }
            } as u8;
        }
        let last_state = self.last_endpoint_state.as_slice();
        let state_offset = self.endpoint_states.len();
        self.endpoint_states.append(last_state).unwrap();
        self.endpoint_state_index.push(state_offset).unwrap();
    }

    pub fn get_item(&mut self, parent: &Option<Item>, index: u64) -> Item {
        use Item::*;
        match parent {
            None => Transfer(self.item_index.get(index).unwrap()),
            Some(Transfer(transfer_index_id)) =>
                Transaction(*transfer_index_id, {
                    let entry = self.transfer_index.get(*transfer_index_id).unwrap();
                    let endpoint_id = entry.endpoint_id() as usize;
                    let transfer_id = entry.transfer_id();
                    let ep_data = &mut self.endpoint_data[endpoint_id];
                    let offset = ep_data.transfer_index.get(transfer_id).unwrap();
                    ep_data.transaction_ids.get(offset + index).unwrap()
                }),
            Some(Transaction(transfer_index_id, transaction_id)) =>
                Packet(*transfer_index_id, *transaction_id, {
                    self.transaction_index.get(*transaction_id).unwrap() + index}),
            Some(Packet(..)) => panic!("packets do not have children"),
        }
    }

    fn item_range(&mut self, item: &Item) -> Range<u64> {
        use Item::*;
        match item {
            Transfer(transfer_index_id) => {
                let entry = self.transfer_index.get(*transfer_index_id).unwrap();
                let endpoint_id = entry.endpoint_id() as usize;
                let transfer_id = entry.transfer_id();
                let ep_data = &mut self.endpoint_data[endpoint_id];
                get_index_range(&mut ep_data.transfer_index,
                    ep_data.transaction_ids.len(), transfer_id)
            },
            Transaction(_, transaction_id) => {
                get_index_range(&mut self.transaction_index,
                    self.packet_index.len(), *transaction_id)
            },
            Packet(.., packet_id) => {
                get_index_range(&mut self.packet_index,
                    self.packet_data.len(), *packet_id)
            },
        }
    }

    pub fn item_count(&mut self, parent: &Option<Item>) -> u64 {
        use Item::*;
        match parent {
            None => self.item_index.len(),
            Some(item) => match item {
                Transfer(id) => {
                    let entry = self.transfer_index.get(*id).unwrap();
                    if entry.is_start() {
                        let range = self.item_range(&item);
                        range.end - range.start
                    } else {
                        0
                    }
                },
                Transaction(..) => {
                    let range = self.item_range(&item);
                    range.end - range.start
                },
                Packet(..) => 0,
            }
        }
    }

    pub fn get_summary(&mut self, item: &Item) -> String {
        use Item::*;
        match item {
            Packet(.., packet_id) => {
                let packet = self.get_packet(*packet_id);
                let pid = PID::from(packet[0]);
                format!("{} packet{}: {:02X?}",
                    pid,
                    match PacketFields::from_packet(&packet) {
                        PacketFields::SOF(sof) => format!(
                            " with frame number {}, CRC {:02X}",
                            sof.frame_number(),
                            sof.crc()),
                        PacketFields::Token(token) => format!(
                            " on {}.{}, CRC {:02X}",
                            token.device_address(),
                            token.endpoint_number(),
                            token.crc()),
                        PacketFields::Data(data) => format!(
                            " with {} data bytes and CRC {:04X}",
                            packet.len() - 3,
                            data.crc),
                        PacketFields::None => "".to_string()
                    },
                    packet)
            },
            Transaction(_, transaction_id) => {
                let (range, payload_size) =
                    self.get_transaction_stats(transaction_id);
                let pid = self.get_packet_pid(range.start);
                let count = range.end - range.start;
                match (pid, payload_size) {
                    (PID::SOF, _) => format!(
                        "{} SOF packets", count),
                    (_, None) => format!(
                        "{} transaction, {} packets", pid, count),
                    (_, Some(size)) => format!(
                        "{} transaction, {} packets with {} data bytes",
                        pid, count, size)
                }
            },
            Transfer(transfer_index_id) => {
                let entry = self.transfer_index.get(*transfer_index_id).unwrap();
                let endpoint_id = entry.endpoint_id();
                let endpoint = self.endpoints.get(endpoint_id as u64).unwrap();
                if !entry.is_start() {
                    let ep_data = &mut self.endpoint_data[endpoint_id as usize];
                    return format!("End of {}", match ep_data.ep_type {
                        EndpointType::Invalid => "invalid groups".to_string(),
                        EndpointType::Framing => "SOF groups".to_string(),
                        EndpointType::Control => format!(
                            "control transfer on device {}",
                            endpoint.device_address),
                        EndpointType::Normal => format!(
                            "bulk transfer on endpoint {}.{}",
                            endpoint.device_address, endpoint.endpoint_number)
                    });
                }
                let range = self.item_range(&item);
                let count = range.end - range.start;
                let ep_data = &mut self.endpoint_data[endpoint_id as usize];
                match ep_data.ep_type {
                    EndpointType::Invalid => format!(
                        "{} invalid groups", count),
                    EndpointType::Framing => format!(
                        "{} SOF groups", count),
                    EndpointType::Control => {
                        use RequestType::*;
                        use Recipient::*;
                        use Direction::*;
                        use PID::*;
                        let transaction_ids =
                            ep_data.transaction_ids.get_range(range).unwrap();
                        let setup_transaction_id = transaction_ids[0];
                        let setup_packet_id =
                            self.transaction_index.get(setup_transaction_id)
                                                  .unwrap();
                        let data_packet_id = setup_packet_id + 1;
                        let data_packet = self.get_packet(data_packet_id);
                        let fields = SetupFields::from_data_packet(&data_packet);
                        let request_type = fields.type_fields.request_type();
                        let direction = fields.type_fields.direction();
                        let request = fields.request;
                        let action = match direction {
                            In => "reading",
                            Out => "writing"
                        };
                        let data_size = transaction_ids.iter().map(|id| {
                            let (range, payload) = self.get_transaction_stats(id);
                            let pid = self.get_packet_pid(range.start);
                            match (direction, pid, payload) {
                                (In, IN, Some(size)) => size,
                                (Out, OUT, Some(size)) => size,
                                (..) => 0,
                            }
                        }).sum();
                        format!(
                            "{} for {}{}",
                            match request_type {
                                Standard => {
                                    let std_req = StandardRequest::from(request);
                                    std_req.description(&fields)
                                },
                                _ => format!(
                                    "{:?} request #{}, index {}, value {}",
                                    request_type, request,
                                    fields.index, fields.value)
                            },
                            match fields.type_fields.recipient() {
                                Device => format!("device {}",
                                                  endpoint.device_address),
                                Interface => format!("interface {}.{}",
                                                     endpoint.device_address,
                                                     fields.index),
                                Endpoint => format!("endpoint {}.{} {}",
                                                    endpoint.device_address,
                                                    fields.index & 0x7F,
                                                    if (fields.index & 0x80) == 0 {
                                                        "OUT"
                                                    } else {
                                                        "IN"
                                                    }),
                                _ => format!("device {}, index {}",
                                             endpoint.device_address,
                                             fields.index)
                            },
                            match (fields.length, data_size) {
                                (0, 0) => "".to_string(),
                                (len, size) if size == len as usize => format!(
                                    ", {} {} bytes", action, len),
                                (len, size) => format!(
                                    ", {} {} of {} requested bytes",
                                    action, size, len)
                            }
                        )
                    },
                    EndpointType::Normal => format!(
                        "Bulk transfer with {} transactions on endpoint {}.{}",
                        count, endpoint.device_address, endpoint.endpoint_number)
                }
            }
        }
    }

    pub fn get_connectors(&mut self, item: &Item) -> String {
        use EndpointState::*;
        use Item::*;
        let endpoint_count = self.endpoints.len() as usize;
        const MIN_LEN: usize = " └─".len();
        let string_length = MIN_LEN + endpoint_count;
        let mut connectors = String::with_capacity(string_length);
        let transfer_index_id = match item {
            Transfer(i) | Transaction(i, _) | Packet(i, ..) => i
        };
        let entry = self.transfer_index.get(*transfer_index_id).unwrap();
        let endpoint_id = entry.endpoint_id() as usize;
        let endpoint_state = self.get_endpoint_state(*transfer_index_id);
        let state_length = endpoint_state.len();
        let extended = self.transfer_extended(endpoint_id, *transfer_index_id);
        let ep_data = &mut self.endpoint_data[endpoint_id];
        let last_transaction = match item {
            Transaction(_, transaction_id) | Packet(_, transaction_id, _) => {
                let range = get_index_range(&mut ep_data.transfer_index,
                    ep_data.transaction_ids.len(), entry.transfer_id());
                let last_transaction_id =
                    ep_data.transaction_ids.get(range.end - 1).unwrap();
                *transaction_id == last_transaction_id
            }, _ => false
        };
        let last_packet = match item {
            Packet(_, transaction_id, packet_id) => {
                let range = get_index_range(&mut self.transaction_index,
                    self.packet_index.len(), *transaction_id);
                *packet_id == range.end - 1
            }, _ => false
        };
        let last = last_transaction && !extended;
        let mut thru = false;
        for i in 0..state_length {
            let state = EndpointState::from(endpoint_state[i]);
            let active = state != Idle;
            let on_endpoint = i == endpoint_id;
            thru |= match (item, state, on_endpoint) {
                (Transfer(..), Starting | Ending, _) => true,
                (Transaction(..) | Packet(..), _, true) => on_endpoint,
                _ => false,
            };
            connectors.push(match item {
                Transfer(..) => {
                    match (state, thru) {
                        (Idle,     _    ) => ' ',
                        (Starting, _    ) => '○',
                        (Ongoing,  false) => '│',
                        (Ongoing,  true ) => '┼',
                        (Ending,   _    ) => '└',
                    }
                },
                Transaction(..) => {
                    match (on_endpoint, active, thru, last) {
                        (false, false, false, _    ) => ' ',
                        (false, false, true,  _    ) => '─',
                        (false, true,  false, _    ) => '│',
                        (false, true,  true,  _    ) => '┼',
                        (true,  _,     _,     false) => '├',
                        (true,  _,     _,     true ) => '└',
                    }
                },
                Packet(..) => {
                    match (on_endpoint, active, last) {
                        (false, false, _    ) => ' ',
                        (false, true,  _    ) => '│',
                        (true,  _,     false) => '│',
                        (true,  _,     true ) => ' ',
                    }
                }
            });
        };
        for _ in state_length..endpoint_count {
            connectors.push(match item {
                Transfer(..)    => '─',
                Transaction(..) => '─',
                Packet(..)      => ' ',
            });
        }
        connectors.push_str(
            match (item, last_packet) {
                (Transfer(_), _) if entry.is_start() => "─",
                (Transfer(_), _)                     => "──□ ",
                (Transaction(..), _)                 => "───",
                (Packet(..), false)                  => "    ├──",
                (Packet(..), true)                   => "    └──",
            }
        );
        connectors
    }

    fn transfer_extended(&mut self, endpoint_id: usize, index: u64) -> bool {
        use EndpointState::*;
        let count = self.transfer_index.len();
        if index + 1 >= count {
            return false;
        };
        let state = self.get_endpoint_state(index + 1);
        if endpoint_id >= state.len() {
            false
        } else {
            match EndpointState::from(state[endpoint_id]) {
                Ongoing => true,
                _ => false,
            }
        }
    }

    fn get_endpoint_state(&mut self, index: u64) -> Vec<u8> {
        let range = get_index_range(
            &mut self.endpoint_state_index,
            self.endpoint_states.len(), index);
        self.endpoint_states.get_range(range).unwrap()
    }

    fn get_packet(&mut self, index: u64) -> Vec<u8> {
        let range = get_index_range(&mut self.packet_index,
                                    self.packet_data.len(), index);
        self.packet_data.get_range(range).unwrap()
    }

    fn get_packet_pid(&mut self, index: u64) -> PID {
        let offset = self.packet_index.get(index).unwrap();
        PID::from(self.packet_data.get(offset).unwrap())
    }

    fn get_transaction_stats(&mut self, index: &u64) -> (Range<u64>, Option<usize>) {
        let range = get_index_range(&mut self.transaction_index,
                                    self.packet_index.len(), *index);
        let packet_count = range.end - range.start;
        let pid = self.get_packet_pid(range.start);
        use PID::*;
        let payload_size = match pid {
            IN | OUT if packet_count >= 2 => {
                let data_packet_id = range.start + 1;
                let data_packet = self.get_packet(data_packet_id);
                match PID::from(data_packet[0]) {
                    DATA0 | DATA1 => Some(data_packet.len() - 3),
                    _ => None
                }
            },
            _ => None
        };
        (range, payload_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sof() {
        let p = PacketFields::from_packet(&vec![0xa5, 0xde, 0x1e]);
        if let PacketFields::SOF(sof) = p {
            assert!(sof.frame_number() == 1758);
            assert!(sof.crc() == 0x03);
        } else {
            panic!("Expected SOF but got {:?}", p);
        }

    }

    #[test]
    fn test_parse_setup() {
        let p = PacketFields::from_packet(&vec![0x2d, 0x02, 0xa8]);
        if let PacketFields::Token(tok) = p {
            assert!(tok.device_address() == 2);
            assert!(tok.endpoint_number() == 0);
            assert!(tok.crc() == 0x15);
        } else {
            panic!("Expected Token but got {:?}", p);
        }

    }

    #[test]
    fn test_parse_in() {
        let p = PacketFields::from_packet(&vec![0x69, 0x82, 0x18]);
        if let PacketFields::Token(tok) = p {
            assert!(tok.device_address() == 2);
            assert!(tok.endpoint_number() == 1);
            assert!(tok.crc() == 0x03);
        } else {
            panic!("Expected Token but got {:?}", p);
        }

    }

    #[test]
    fn test_parse_data() {
        let p = PacketFields::from_packet(&vec![0xc3, 0x40, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0xaa, 0xd5]);
        if let PacketFields::Data(data) = p {
            assert!(data.crc == 0xd5aa);
        } else {
            panic!("Expected Data but got {:?}", p);
        }

    }
}

