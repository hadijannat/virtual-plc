//! EtherCAT slave configuration and PDO mapping.
//!
//! Provides structures for:
//! - Slave identification and state management
//! - PDO (Process Data Object) mapping for cyclic data
//! - SDO (Service Data Object) access for configuration
//! - ESI (EtherCAT Slave Information) parsing support

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// EtherCAT slave state machine states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum SlaveState {
    /// Initial state after power-on.
    #[default]
    Init = 0x01,
    /// Pre-operational: SDO communication available.
    PreOp = 0x02,
    /// Safe-operational: inputs active, outputs safe.
    SafeOp = 0x04,
    /// Operational: full I/O active.
    Op = 0x08,
    /// Bootstrap: firmware update mode.
    Bootstrap = 0x03,
}

impl SlaveState {
    /// Parse state from raw EtherCAT AL status register.
    pub fn from_al_status(status: u8) -> Option<Self> {
        match status & 0x0F {
            0x01 => Some(Self::Init),
            0x02 => Some(Self::PreOp),
            0x03 => Some(Self::Bootstrap),
            0x04 => Some(Self::SafeOp),
            0x08 => Some(Self::Op),
            _ => None,
        }
    }

    /// Get the AL control value for requesting this state.
    pub fn to_al_control(self) -> u8 {
        self as u8
    }
}

impl std::fmt::Display for SlaveState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Init => write!(f, "INIT"),
            Self::PreOp => write!(f, "PRE_OP"),
            Self::SafeOp => write!(f, "SAFE_OP"),
            Self::Op => write!(f, "OP"),
            Self::Bootstrap => write!(f, "BOOTSTRAP"),
        }
    }
}

/// EtherCAT slave identification.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SlaveIdentity {
    /// Vendor ID from ESC EEPROM.
    pub vendor_id: u32,
    /// Product code from ESC EEPROM.
    pub product_code: u32,
    /// Revision number.
    pub revision: u32,
    /// Serial number (if available).
    pub serial: u32,
}

impl SlaveIdentity {
    /// Create a new slave identity.
    pub fn new(vendor_id: u32, product_code: u32, revision: u32, serial: u32) -> Self {
        Self {
            vendor_id,
            product_code,
            revision,
            serial,
        }
    }
}

impl std::fmt::Display for SlaveIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "V:{:#010x} P:{:#010x} R:{:#010x}",
            self.vendor_id, self.product_code, self.revision
        )
    }
}

/// PDO entry describing a single data item in a PDO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdoEntry {
    /// CoE object index.
    pub index: u16,
    /// CoE object subindex.
    pub subindex: u8,
    /// Bit length of this entry.
    pub bit_length: u16,
    /// Human-readable name (from ESI).
    pub name: String,
    /// Data type name (e.g., "BOOL", "UINT16").
    pub data_type: String,
}

impl PdoEntry {
    /// Create a new PDO entry.
    pub fn new(index: u16, subindex: u8, bit_length: u16) -> Self {
        Self {
            index,
            subindex,
            bit_length,
            name: String::new(),
            data_type: String::new(),
        }
    }

    /// Set the entry name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the data type.
    pub fn with_data_type(mut self, data_type: impl Into<String>) -> Self {
        self.data_type = data_type.into();
        self
    }

    /// Calculate byte offset and bit offset within the PDO.
    pub fn byte_offset(&self, bit_offset: usize) -> (usize, usize) {
        (bit_offset / 8, bit_offset % 8)
    }
}

/// PDO (Process Data Object) mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdoMapping {
    /// PDO index (e.g., 0x1600 for RxPDO, 0x1A00 for TxPDO).
    pub index: u16,
    /// Entries in this PDO.
    pub entries: Vec<PdoEntry>,
    /// Total bit length of this PDO.
    pub total_bits: u16,
    /// Whether this is a TxPDO (slave→master) or RxPDO (master→slave).
    pub is_tx: bool,
}

impl PdoMapping {
    /// Create a new PDO mapping.
    pub fn new(index: u16, is_tx: bool) -> Self {
        Self {
            index,
            entries: Vec::new(),
            total_bits: 0,
            is_tx,
        }
    }

    /// Add an entry to the PDO.
    pub fn add_entry(&mut self, entry: PdoEntry) {
        self.total_bits += entry.bit_length;
        self.entries.push(entry);
    }

    /// Get total byte length (rounded up).
    pub fn byte_length(&self) -> usize {
        (self.total_bits as usize + 7) / 8
    }
}

/// Sync Manager configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncManager {
    /// SM index (0-7).
    pub index: u8,
    /// Physical start address in ESC memory.
    pub start_address: u16,
    /// Length in bytes.
    pub length: u16,
    /// Control register value.
    pub control: u8,
    /// SM type.
    pub sm_type: SyncManagerType,
    /// Associated PDOs.
    pub pdos: Vec<u16>,
}

/// Sync Manager types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SyncManagerType {
    /// Unused.
    #[default]
    Unused,
    /// Mailbox output (master→slave).
    MailboxOut,
    /// Mailbox input (slave→master).
    MailboxIn,
    /// Process data output (master→slave).
    ProcessDataOut,
    /// Process data input (slave→master).
    ProcessDataIn,
}

/// Complete slave configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlaveConfig {
    /// Position in the EtherCAT ring (0-based).
    pub position: u16,
    /// Configured station address.
    pub configured_address: u16,
    /// Slave identity.
    pub identity: SlaveIdentity,
    /// Current state.
    pub state: SlaveState,
    /// Sync Manager configurations.
    pub sync_managers: Vec<SyncManager>,
    /// TxPDO mappings (slave→master).
    pub tx_pdos: Vec<PdoMapping>,
    /// RxPDO mappings (master→slave).
    pub rx_pdos: Vec<PdoMapping>,
    /// Offset in the input process image (bytes).
    pub input_offset: usize,
    /// Size of input data (bytes).
    pub input_size: usize,
    /// Offset in the output process image (bytes).
    pub output_offset: usize,
    /// Size of output data (bytes).
    pub output_size: usize,
    /// Whether DC (Distributed Clocks) is supported.
    pub dc_supported: bool,
    /// Human-readable name.
    pub name: String,
}

impl SlaveConfig {
    /// Create a new slave configuration.
    pub fn new(position: u16, identity: SlaveIdentity) -> Self {
        Self {
            position,
            configured_address: 0x1000 + position,
            identity,
            state: SlaveState::Init,
            sync_managers: Vec::new(),
            tx_pdos: Vec::new(),
            rx_pdos: Vec::new(),
            input_offset: 0,
            input_size: 0,
            output_offset: 0,
            output_size: 0,
            dc_supported: false,
            name: String::new(),
        }
    }

    /// Calculate total input size from TxPDOs.
    pub fn calculate_input_size(&self) -> usize {
        self.tx_pdos.iter().map(|p| p.byte_length()).sum()
    }

    /// Calculate total output size from RxPDOs.
    pub fn calculate_output_size(&self) -> usize {
        self.rx_pdos.iter().map(|p| p.byte_length()).sum()
    }
}

/// SDO (Service Data Object) address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SdoAddress {
    /// Object index.
    pub index: u16,
    /// Object subindex.
    pub subindex: u8,
}

impl SdoAddress {
    /// Create a new SDO address.
    pub const fn new(index: u16, subindex: u8) -> Self {
        Self { index, subindex }
    }
}

impl std::fmt::Display for SdoAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#06x}:{}", self.index, self.subindex)
    }
}

/// SDO data types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdoDataType {
    Bool,
    Int8,
    Int16,
    Int32,
    Int64,
    Uint8,
    Uint16,
    Uint32,
    Uint64,
    Real32,
    Real64,
    VisibleString,
    OctetString,
}

impl SdoDataType {
    /// Get the byte size of this data type (0 for variable-length).
    pub fn byte_size(&self) -> usize {
        match self {
            Self::Bool | Self::Int8 | Self::Uint8 => 1,
            Self::Int16 | Self::Uint16 => 2,
            Self::Int32 | Self::Uint32 | Self::Real32 => 4,
            Self::Int64 | Self::Uint64 | Self::Real64 => 8,
            Self::VisibleString | Self::OctetString => 0,
        }
    }
}

/// SDO request for reading or writing configuration data.
#[derive(Debug, Clone)]
pub struct SdoRequest {
    /// Target slave position.
    pub slave: u16,
    /// SDO address.
    pub address: SdoAddress,
    /// Data to write (None for read requests).
    pub write_data: Option<Vec<u8>>,
    /// Complete access flag (read/write entire object).
    pub complete_access: bool,
}

impl SdoRequest {
    /// Create a read request.
    pub fn read(slave: u16, index: u16, subindex: u8) -> Self {
        Self {
            slave,
            address: SdoAddress::new(index, subindex),
            write_data: None,
            complete_access: false,
        }
    }

    /// Create a write request.
    pub fn write(slave: u16, index: u16, subindex: u8, data: Vec<u8>) -> Self {
        Self {
            slave,
            address: SdoAddress::new(index, subindex),
            write_data: Some(data),
            complete_access: false,
        }
    }

    /// Enable complete access mode.
    pub fn with_complete_access(mut self) -> Self {
        self.complete_access = true;
        self
    }
}

/// Common SDO addresses for standard objects.
pub mod sdo_addresses {
    use super::SdoAddress;

    /// Device type.
    pub const DEVICE_TYPE: SdoAddress = SdoAddress::new(0x1000, 0);
    /// Error register.
    pub const ERROR_REGISTER: SdoAddress = SdoAddress::new(0x1001, 0);
    /// Manufacturer device name.
    pub const DEVICE_NAME: SdoAddress = SdoAddress::new(0x1008, 0);
    /// Hardware version.
    pub const HW_VERSION: SdoAddress = SdoAddress::new(0x1009, 0);
    /// Software version.
    pub const SW_VERSION: SdoAddress = SdoAddress::new(0x100A, 0);
    /// Identity object.
    pub const IDENTITY: SdoAddress = SdoAddress::new(0x1018, 0);

    /// RxPDO mapping base (0x1600-0x17FF).
    pub const RXPDO_MAPPING_BASE: u16 = 0x1600;
    /// TxPDO mapping base (0x1A00-0x1BFF).
    pub const TXPDO_MAPPING_BASE: u16 = 0x1A00;

    /// SM2 PDO assignment (outputs).
    pub const SM2_PDO_ASSIGN: SdoAddress = SdoAddress::new(0x1C12, 0);
    /// SM3 PDO assignment (inputs).
    pub const SM3_PDO_ASSIGN: SdoAddress = SdoAddress::new(0x1C13, 0);
}

/// Network configuration containing all discovered slaves.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Discovered slaves indexed by position.
    pub slaves: HashMap<u16, SlaveConfig>,
    /// Total input process image size.
    pub total_input_size: usize,
    /// Total output process image size.
    pub total_output_size: usize,
    /// Network interface name.
    pub interface: String,
}

impl NetworkConfig {
    /// Create a new empty network configuration.
    pub fn new(interface: impl Into<String>) -> Self {
        Self {
            slaves: HashMap::new(),
            total_input_size: 0,
            total_output_size: 0,
            interface: interface.into(),
        }
    }

    /// Add a slave to the network.
    pub fn add_slave(&mut self, config: SlaveConfig) {
        let position = config.position;
        self.slaves.insert(position, config);
        self.recalculate_offsets();
    }

    /// Recalculate process image offsets after slave changes.
    ///
    /// Call this method after modifying slaves via [`get_slave_mut()`] to ensure
    /// process image offsets are correct. This is called automatically by
    /// [`add_slave()`].
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Modify a slave's PDO mapping
    /// if let Some(slave) = network.get_slave_mut(1) {
    ///     slave.tx_pdos.push(PdoEntry { ... });
    /// }
    /// // Recalculate offsets after modification
    /// network.recalculate_offsets();
    /// ```
    pub fn recalculate_offsets(&mut self) {
        let mut input_offset = 0;
        let mut output_offset = 0;

        // Sort by position for deterministic layout
        let mut positions: Vec<_> = self.slaves.keys().copied().collect();
        positions.sort();

        for pos in positions {
            if let Some(slave) = self.slaves.get_mut(&pos) {
                slave.input_offset = input_offset;
                slave.input_size = slave.calculate_input_size();
                input_offset += slave.input_size;

                slave.output_offset = output_offset;
                slave.output_size = slave.calculate_output_size();
                output_offset += slave.output_size;
            }
        }

        self.total_input_size = input_offset;
        self.total_output_size = output_offset;
    }

    /// Get slave count.
    pub fn slave_count(&self) -> usize {
        self.slaves.len()
    }

    /// Get a slave by position.
    pub fn get_slave(&self, position: u16) -> Option<&SlaveConfig> {
        self.slaves.get(&position)
    }

    /// Get a mutable slave by position.
    pub fn get_slave_mut(&mut self, position: u16) -> Option<&mut SlaveConfig> {
        self.slaves.get_mut(&position)
    }

    /// Clear all slaves and reset sizes.
    ///
    /// This should be called before re-scanning the network to ensure
    /// stale slaves are not retained.
    pub fn clear(&mut self) {
        self.slaves.clear();
        self.total_input_size = 0;
        self.total_output_size = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slave_state_conversion() {
        assert_eq!(SlaveState::from_al_status(0x01), Some(SlaveState::Init));
        assert_eq!(SlaveState::from_al_status(0x02), Some(SlaveState::PreOp));
        assert_eq!(SlaveState::from_al_status(0x04), Some(SlaveState::SafeOp));
        assert_eq!(SlaveState::from_al_status(0x08), Some(SlaveState::Op));
        assert_eq!(SlaveState::from_al_status(0xFF), None);
    }

    #[test]
    fn test_pdo_mapping() {
        let mut pdo = PdoMapping::new(0x1A00, true);
        pdo.add_entry(PdoEntry::new(0x6000, 1, 1).with_name("Input 1"));
        pdo.add_entry(PdoEntry::new(0x6000, 2, 16).with_name("Analog 1"));

        assert_eq!(pdo.total_bits, 17);
        assert_eq!(pdo.byte_length(), 3);
    }

    #[test]
    fn test_slave_identity_display() {
        let id = SlaveIdentity::new(0x00000002, 0x044C2C52, 0x00110001, 0);
        let display = format!("{}", id);
        assert!(display.contains("0x00000002"));
        assert!(display.contains("0x044c2c52"));
    }

    #[test]
    fn test_network_config_offsets() {
        let mut network = NetworkConfig::new("eth0");

        let mut slave0 = SlaveConfig::new(0, SlaveIdentity::new(1, 1, 1, 0));
        slave0.tx_pdos.push({
            let mut pdo = PdoMapping::new(0x1A00, true);
            pdo.add_entry(PdoEntry::new(0x6000, 1, 16));
            pdo
        });
        slave0.rx_pdos.push({
            let mut pdo = PdoMapping::new(0x1600, false);
            pdo.add_entry(PdoEntry::new(0x7000, 1, 8));
            pdo
        });

        let mut slave1 = SlaveConfig::new(1, SlaveIdentity::new(2, 2, 1, 0));
        slave1.tx_pdos.push({
            let mut pdo = PdoMapping::new(0x1A00, true);
            pdo.add_entry(PdoEntry::new(0x6000, 1, 32));
            pdo
        });

        network.add_slave(slave0);
        network.add_slave(slave1);

        assert_eq!(network.slave_count(), 2);
        assert_eq!(network.total_input_size, 6); // 2 + 4 bytes
        assert_eq!(network.total_output_size, 1); // 1 byte

        let s0 = network.get_slave(0).unwrap();
        assert_eq!(s0.input_offset, 0);
        assert_eq!(s0.input_size, 2);

        let s1 = network.get_slave(1).unwrap();
        assert_eq!(s1.input_offset, 2);
        assert_eq!(s1.input_size, 4);
    }

    #[test]
    fn test_sdo_request() {
        let read_req = SdoRequest::read(0, 0x1000, 0);
        assert!(read_req.write_data.is_none());

        let write_req = SdoRequest::write(0, 0x6000, 1, vec![0x01]);
        assert!(write_req.write_data.is_some());
    }
}
