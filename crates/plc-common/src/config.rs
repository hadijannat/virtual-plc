//! Configuration structures for the PLC runtime.
//!
//! Supports TOML deserialization with sensible defaults for
//! development and explicit values for production deployment.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// Top-level runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    /// Cycle time for the main scan loop.
    #[serde(with = "humantime_serde")]
    pub cycle_time: Duration,

    /// Watchdog timeout (typically 2-3x cycle time).
    #[serde(with = "humantime_serde")]
    pub watchdog_timeout: Duration,

    /// Maximum allowed cycle overrun before fault.
    #[serde(with = "humantime_serde")]
    pub max_overrun: Duration,

    /// Path to the compiled Wasm logic module.
    pub wasm_module: Option<PathBuf>,

    /// Real-time configuration.
    pub realtime: RealtimeConfig,

    /// Fieldbus configuration.
    pub fieldbus: FieldbusConfig,

    /// Metrics and diagnostics configuration.
    pub metrics: MetricsConfig,

    /// Fault handling policy.
    pub fault_policy: FaultPolicyConfig,

    /// WebAssembly runtime configuration.
    pub wasm: WasmConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            cycle_time: Duration::from_millis(1),
            watchdog_timeout: Duration::from_millis(3),
            max_overrun: Duration::from_micros(500),
            wasm_module: None,
            realtime: RealtimeConfig::default(),
            fieldbus: FieldbusConfig::default(),
            metrics: MetricsConfig::default(),
            fault_policy: FaultPolicyConfig::default(),
            wasm: WasmConfig::default(),
        }
    }
}

/// Real-time scheduling configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RealtimeConfig {
    /// Enable real-time scheduling (requires privileges).
    pub enabled: bool,

    /// Scheduler policy: "fifo" or "rr" (round-robin).
    pub policy: SchedPolicy,

    /// Scheduler priority (1-99 for RT policies).
    pub priority: u8,

    /// CPU affinity for the RT thread.
    pub cpu_affinity: CpuAffinity,

    /// Lock all memory pages (mlockall).
    pub lock_memory: bool,

    /// Pre-fault stack size in bytes.
    pub prefault_stack_size: usize,

    /// Fail immediately at startup if RT requirements cannot be met.
    /// When true, the runtime will return an error if PREEMPT_RT kernel,
    /// CAP_SYS_NICE, or CAP_IPC_LOCK are not available.
    pub fail_fast: bool,
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            policy: SchedPolicy::Fifo,
            priority: 90,
            cpu_affinity: CpuAffinity::None,
            lock_memory: true,
            prefault_stack_size: 8 * 1024 * 1024, // 8 MiB
            fail_fast: false,
        }
    }
}

/// Policy for handling cycle overruns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OverrunPolicy {
    /// Enter fault state on overrun (strictest).
    #[default]
    Fault,
    /// Log warning but continue execution.
    Warn,
    /// Silently ignore overruns.
    Ignore,
}

/// Policy for setting outputs when entering safe/fault state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SafeOutputPolicy {
    /// Set all outputs to zero/off (safest default).
    #[default]
    AllOff,
    /// Hold last known output values.
    HoldLast,
    /// Use user-defined safe values.
    UserDefined {
        /// Safe values for digital outputs (32-bit words, one per output group).
        digital: Vec<u32>,
        /// Safe values for analog outputs (16-bit signed values).
        analog: Vec<i16>,
    },
}

/// Fault handling policy configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FaultPolicyConfig {
    /// How to handle cycle overruns.
    pub on_overrun: OverrunPolicy,
    /// What to do with outputs when entering safe/fault state.
    pub safe_outputs: SafeOutputPolicy,
    /// Whether faults require manual reset (latch) or auto-recover.
    pub fault_latch: bool,
}

/// Scheduler policy for real-time threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SchedPolicy {
    /// SCHED_FIFO: First-in-first-out real-time.
    #[default]
    Fifo,
    /// SCHED_RR: Round-robin real-time.
    Rr,
    /// SCHED_OTHER: Normal time-sharing (non-RT).
    Other,
}

/// CPU affinity specification.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CpuAffinity {
    /// No affinity set (OS chooses).
    #[default]
    None,
    /// Pin to a single CPU core.
    Single(usize),
    /// Pin to a set of CPU cores.
    Set(Vec<usize>),
}

impl Serialize for CpuAffinity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            CpuAffinity::None => serializer.serialize_none(),
            CpuAffinity::Single(cpu) => serializer.serialize_u64(*cpu as u64),
            CpuAffinity::Set(cpus) => cpus.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for CpuAffinity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, Visitor};

        struct CpuAffinityVisitor;

        impl<'de> Visitor<'de> for CpuAffinityVisitor {
            type Value = CpuAffinity;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("null, an integer, or an array of integers")
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(CpuAffinity::None)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(CpuAffinity::None)
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(CpuAffinity::Single(value as usize))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value < 0 {
                    return Err(de::Error::custom("CPU index cannot be negative"));
                }
                Ok(CpuAffinity::Single(value as usize))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let mut cpus = Vec::new();
                while let Some(cpu) = seq.next_element::<usize>()? {
                    cpus.push(cpu);
                }
                Ok(CpuAffinity::Set(cpus))
            }
        }

        deserializer.deserialize_any(CpuAffinityVisitor)
    }
}

/// Fieldbus configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldbusConfig {
    /// Fieldbus driver type.
    pub driver: FieldbusDriver,

    /// EtherCAT-specific configuration.
    pub ethercat: Option<EthercatConfig>,

    /// Modbus TCP configuration.
    pub modbus: Option<ModbusConfig>,
}

impl Default for FieldbusConfig {
    fn default() -> Self {
        Self {
            driver: FieldbusDriver::Simulated,
            ethercat: None,
            modbus: None,
        }
    }
}

/// Supported fieldbus drivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FieldbusDriver {
    /// Simulated I/O for testing.
    #[default]
    Simulated,
    /// EtherCAT via SOEM.
    EtherCAT,
    /// Modbus TCP.
    #[serde(rename = "modbus_tcp")]
    ModbusTcp,
}

/// EtherCAT-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EthercatConfig {
    /// Network interface name (e.g., "enp3s0", "eth0").
    /// Must be explicitly configured - no default to avoid using wrong interface.
    pub interface: Option<String>,

    /// Enable Distributed Clocks synchronization.
    pub dc_enabled: bool,

    /// DC sync0 cycle time.
    #[serde(with = "humantime_serde")]
    pub dc_sync0_cycle: Duration,

    /// Path to ESI (EtherCAT Slave Information) files.
    pub esi_path: Option<PathBuf>,
}

impl Default for EthercatConfig {
    fn default() -> Self {
        Self {
            interface: None, // Must be explicitly configured
            dc_enabled: true,
            dc_sync0_cycle: Duration::from_millis(1),
            esi_path: None,
        }
    }
}

/// Modbus TCP configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModbusConfig {
    /// Server address (host:port).
    pub address: String,

    /// Slave ID / Unit identifier.
    pub unit_id: u8,

    /// Connection timeout.
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
}

impl Default for ModbusConfig {
    fn default() -> Self {
        Self {
            address: String::from("127.0.0.1:502"),
            unit_id: 1,
            timeout: Duration::from_secs(1),
        }
    }
}

/// Metrics and diagnostics configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    /// Enable metrics collection.
    pub enabled: bool,

    /// Size of the latency histogram ring buffer.
    pub histogram_size: usize,

    /// Percentiles to compute (e.g., [50, 90, 99, 99.9]).
    pub percentiles: Vec<f64>,

    /// Export metrics via HTTP endpoint.
    pub http_export: bool,

    /// HTTP export port.
    pub http_port: u16,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            histogram_size: 10_000,
            percentiles: vec![50.0, 90.0, 99.0, 99.9, 99.99],
            http_export: false,
            http_port: 9090,
        }
    }
}

/// WebAssembly runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WasmConfig {
    /// Maximum linear memory size in bytes.
    pub max_memory_bytes: usize,

    /// Maximum epochs (timeout units) per cycle.
    /// Higher values allow longer-running Wasm code but reduce responsiveness.
    pub max_epochs_per_cycle: u64,

    /// Maximum table elements (function pointers, etc.).
    pub max_table_elements: u32,

    /// Enable SIMD instructions in Wasm modules.
    pub enable_simd: bool,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: 16 * 1024 * 1024, // 16 MB
            max_epochs_per_cycle: 100,
            max_table_elements: 10_000,
            enable_simd: false,
        }
    }
}

impl RuntimeConfig {
    /// Load configuration from a TOML file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn from_file(path: &std::path::Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        Self::from_toml(&content)
    }

    /// Parse configuration from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns an error if the TOML is invalid.
    pub fn from_toml(content: &str) -> Result<Self, ConfigError> {
        toml::from_str(content).map_err(ConfigError::Parse)
    }

    /// Serialize configuration to TOML string.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self).map_err(ConfigError::Serialize)
    }
}

/// Configuration-related errors.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// File I/O error.
    #[error("failed to read config file {path}: {source}")]
    Io {
        /// Path to the configuration file.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// TOML parsing error.
    #[error("failed to parse TOML: {0}")]
    Parse(#[from] toml::de::Error),

    /// TOML serialization error.
    #[error("failed to serialize TOML: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Serde helper module for `Duration` using humantime format.
mod humantime_serde {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = humantime::format_duration(*duration).to_string();
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        humantime::parse_duration(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RuntimeConfig::default();
        assert_eq!(config.cycle_time, Duration::from_millis(1));
        assert!(!config.realtime.enabled);
        assert_eq!(config.realtime.priority, 90);
    }

    #[test]
    fn test_parse_toml() {
        let toml = r#"
            cycle_time = "1ms"
            watchdog_timeout = "3ms"

            [realtime]
            enabled = true
            priority = 95
            policy = "fifo"

            [fieldbus]
            driver = "ethercat"

            [fieldbus.ethercat]
            interface = "enp3s0"
            dc_enabled = true
        "#;

        let config = RuntimeConfig::from_toml(toml).unwrap();
        assert_eq!(config.cycle_time, Duration::from_millis(1));
        assert!(config.realtime.enabled);
        assert_eq!(config.realtime.priority, 95);
        assert_eq!(config.fieldbus.driver, FieldbusDriver::EtherCAT);
        // Verify interface is parsed as Some
        assert_eq!(
            config.fieldbus.ethercat.as_ref().unwrap().interface,
            Some("enp3s0".to_string())
        );
    }

    #[test]
    fn test_cpu_affinity_variants() {
        let single: CpuAffinity = serde_json::from_str("3").unwrap();
        assert_eq!(single, CpuAffinity::Single(3));

        let set: CpuAffinity = serde_json::from_str("[1, 2, 3]").unwrap();
        assert_eq!(set, CpuAffinity::Set(vec![1, 2, 3]));
    }

    #[test]
    fn test_roundtrip_toml() {
        let config = RuntimeConfig::default();
        let toml = config.to_toml().unwrap();
        let parsed = RuntimeConfig::from_toml(&toml).unwrap();
        assert_eq!(config.cycle_time, parsed.cycle_time);
    }

    #[test]
    fn test_modbus_tcp_driver_name() {
        // Test that modbus_tcp is the correct TOML name (with underscore)
        let toml = r#"
            [fieldbus]
            driver = "modbus_tcp"
        "#;

        let config = RuntimeConfig::from_toml(toml).unwrap();
        assert_eq!(config.fieldbus.driver, FieldbusDriver::ModbusTcp);

        // Test serialization produces correct name
        let mut config = RuntimeConfig::default();
        config.fieldbus.driver = FieldbusDriver::ModbusTcp;
        let serialized = config.to_toml().unwrap();
        assert!(
            serialized.contains("modbus_tcp"),
            "Expected 'modbus_tcp' in serialized TOML: {}",
            serialized
        );
    }
}
