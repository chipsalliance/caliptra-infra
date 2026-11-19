// Licensed under the Apache-2.0 license

use std::collections::BTreeSet;
use std::env;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{MANIFEST_SCHEMA_VERSION, Manifest};

pub const DEFAULT_VIVADO_SETTINGS: &str = "/vck190-tools/Vivado/2024.2/settings64.sh";
pub const DEFAULT_VIVADO_BIN: &str = "vivado";

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ValueEnum,
)]
pub enum Release {
    #[value(name = "1.x")]
    #[serde(rename = "1.x")]
    V1x,
    #[value(name = "2.0")]
    #[serde(rename = "2.0")]
    V2_0,
    #[value(name = "2.1")]
    #[serde(rename = "2.1")]
    V2_1,
    #[value(name = "2.2")]
    #[serde(rename = "2.2")]
    V2_2,
}

impl Release {
    pub const ALL: &'static [Release] = &[Self::V1x, Self::V2_0, Self::V2_1, Self::V2_2];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::V1x => "1.x",
            Self::V2_0 => "2.0",
            Self::V2_1 => "2.1",
            Self::V2_2 => "2.2",
        }
    }

    pub const fn hw_prefix(&self) -> &'static str {
        match self {
            Self::V1x => "hw/1.x/",
            Self::V2_0 => "hw/2.0/",
            Self::V2_1 => "hw/2.1/",
            Self::V2_2 => "hw/2.2/",
        }
    }

    pub const fn ss_submodule_path(&self) -> Option<&'static str> {
        match self {
            Self::V1x => None,
            Self::V2_0 => Some("hw/2.0/caliptra-ss"),
            Self::V2_1 => Some("hw/2.1/caliptra-ss"),
            Self::V2_2 => Some("hw/2.2/caliptra-ss"),
        }
    }
}

impl fmt::Display for Release {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Release {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "1.x" => Ok(Self::V1x),
            "2.0" => Ok(Self::V2_0),
            "2.1" => Ok(Self::V2_1),
            "2.2" => Ok(Self::V2_2),
            _ => bail!("unknown release '{s}'"),
        }
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ValueEnum,
)]
pub enum Board {
    #[value(name = "zcu104")]
    #[serde(rename = "zcu104")]
    Zcu104,
    #[value(name = "vck190")]
    #[serde(rename = "vck190")]
    Vck190,
}

impl Board {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Zcu104 => "zcu104",
            Self::Vck190 => "vck190",
        }
    }
}

impl fmt::Display for Board {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Board {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "zcu104" => Ok(Self::Zcu104),
            "vck190" => Ok(Self::Vck190),
            _ => bail!("unknown board '{s}'"),
        }
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ValueEnum,
)]
pub enum Subtarget {
    #[value(name = "1.x")]
    #[serde(rename = "1.x")]
    V1x,
    #[value(name = "2.0/core")]
    #[serde(rename = "2.0/core")]
    V2_0Core,
    #[value(name = "2.0/subsystem")]
    #[serde(rename = "2.0/subsystem")]
    V2_0Subsystem,
    #[value(name = "2.1/core")]
    #[serde(rename = "2.1/core")]
    V2_1Core,
    #[value(name = "2.1/subsystem")]
    #[serde(rename = "2.1/subsystem")]
    V2_1Subsystem,
    #[value(name = "2.2/core")]
    #[serde(rename = "2.2/core")]
    V2_2Core,
    #[value(name = "2.2/subsystem")]
    #[serde(rename = "2.2/subsystem")]
    V2_2Subsystem,
}

impl Subtarget {
    pub const ALL: &'static [Subtarget] = &[
        Self::V1x,
        Self::V2_0Core,
        Self::V2_0Subsystem,
        Self::V2_1Core,
        Self::V2_1Subsystem,
        Self::V2_2Core,
        Self::V2_2Subsystem,
    ];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::V1x => "1.x",
            Self::V2_0Core => "2.0/core",
            Self::V2_0Subsystem => "2.0/subsystem",
            Self::V2_1Core => "2.1/core",
            Self::V2_1Subsystem => "2.1/subsystem",
            Self::V2_2Core => "2.2/core",
            Self::V2_2Subsystem => "2.2/subsystem",
        }
    }

    pub const fn hw_prefix(&self) -> &'static str {
        match self {
            Self::V1x => "hw/1.x/",
            Self::V2_0Core => "hw/2.0/core/",
            Self::V2_0Subsystem => "hw/2.0/subsystem/",
            Self::V2_1Core => "hw/2.1/core/",
            Self::V2_1Subsystem => "hw/2.1/subsystem/",
            Self::V2_2Core => "hw/2.2/core/",
            Self::V2_2Subsystem => "hw/2.2/subsystem/",
        }
    }

    pub const fn release(&self) -> Release {
        match self {
            Self::V1x => Release::V1x,
            Self::V2_0Core | Self::V2_0Subsystem => Release::V2_0,
            Self::V2_1Core | Self::V2_1Subsystem => Release::V2_1,
            Self::V2_2Core | Self::V2_2Subsystem => Release::V2_2,
        }
    }
}

impl fmt::Display for Subtarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Subtarget {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "1.x" => Ok(Self::V1x),
            "2.0/core" => Ok(Self::V2_0Core),
            "2.0/subsystem" => Ok(Self::V2_0Subsystem),
            "2.1/core" => Ok(Self::V2_1Core),
            "2.1/subsystem" => Ok(Self::V2_1Subsystem),
            "2.2/core" => Ok(Self::V2_2Core),
            "2.2/subsystem" => Ok(Self::V2_2Subsystem),
            _ => bail!("unknown subtarget '{s}'"),
        }
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ValueEnum,
)]
pub enum Variant {
    #[value(name = "core-hw1.0-itrng")]
    #[serde(rename = "core-hw1.0-itrng")]
    CoreHw1_0Itrng,
    #[value(name = "core-hw1.0-etrng")]
    #[serde(rename = "core-hw1.0-etrng")]
    CoreHw1_0Etrng,
    #[value(name = "core-latest-itrng")]
    #[serde(rename = "core-latest-itrng")]
    CoreLatestItrng,
    #[value(name = "core-latest-etrng")]
    #[serde(rename = "core-latest-etrng")]
    CoreLatestEtrng,
    #[value(name = "core-2.0-segmented")]
    #[serde(rename = "core-2.0-segmented")]
    Core2_0Segmented,
    #[value(name = "core-2.0-standard")]
    #[serde(rename = "core-2.0-standard")]
    Core2_0Standard,
    #[value(name = "subsystem-2.0-segmented")]
    #[serde(rename = "subsystem-2.0-segmented")]
    Subsystem2_0Segmented,
    #[value(name = "subsystem-2.0-standard")]
    #[serde(rename = "subsystem-2.0-standard")]
    Subsystem2_0Standard,
    #[value(name = "core-2.1-segmented")]
    #[serde(rename = "core-2.1-segmented")]
    Core2_1Segmented,
    #[value(name = "core-2.1-standard")]
    #[serde(rename = "core-2.1-standard")]
    Core2_1Standard,
    #[value(name = "subsystem-2.1-segmented")]
    #[serde(rename = "subsystem-2.1-segmented")]
    Subsystem2_1Segmented,
    #[value(name = "subsystem-2.1-standard")]
    #[serde(rename = "subsystem-2.1-standard")]
    Subsystem2_1Standard,
    #[value(name = "core-2.2-segmented")]
    #[serde(rename = "core-2.2-segmented")]
    Core2_2Segmented,
    #[value(name = "core-2.2-standard")]
    #[serde(rename = "core-2.2-standard")]
    Core2_2Standard,
    #[value(name = "subsystem-2.2-segmented")]
    #[serde(rename = "subsystem-2.2-segmented")]
    Subsystem2_2Segmented,
    #[value(name = "subsystem-2.2-standard")]
    #[serde(rename = "subsystem-2.2-standard")]
    Subsystem2_2Standard,
}

impl Variant {
    pub const ALL: &'static [Variant] = &[
        Self::CoreHw1_0Itrng,
        Self::CoreHw1_0Etrng,
        Self::CoreLatestItrng,
        Self::CoreLatestEtrng,
        Self::Core2_0Segmented,
        Self::Core2_0Standard,
        Self::Subsystem2_0Segmented,
        Self::Subsystem2_0Standard,
        Self::Core2_1Segmented,
        Self::Core2_1Standard,
        Self::Subsystem2_1Segmented,
        Self::Subsystem2_1Standard,
        Self::Core2_2Segmented,
        Self::Core2_2Standard,
        Self::Subsystem2_2Segmented,
        Self::Subsystem2_2Standard,
    ];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::CoreHw1_0Itrng => "core-hw1.0-itrng",
            Self::CoreHw1_0Etrng => "core-hw1.0-etrng",
            Self::CoreLatestItrng => "core-latest-itrng",
            Self::CoreLatestEtrng => "core-latest-etrng",
            Self::Core2_0Segmented => "core-2.0-segmented",
            Self::Core2_0Standard => "core-2.0-standard",
            Self::Subsystem2_0Segmented => "subsystem-2.0-segmented",
            Self::Subsystem2_0Standard => "subsystem-2.0-standard",
            Self::Core2_1Segmented => "core-2.1-segmented",
            Self::Core2_1Standard => "core-2.1-standard",
            Self::Subsystem2_1Segmented => "subsystem-2.1-segmented",
            Self::Subsystem2_1Standard => "subsystem-2.1-standard",
            Self::Core2_2Segmented => "core-2.2-segmented",
            Self::Core2_2Standard => "core-2.2-standard",
            Self::Subsystem2_2Segmented => "subsystem-2.2-segmented",
            Self::Subsystem2_2Standard => "subsystem-2.2-standard",
        }
    }

    pub const fn spec(self) -> &'static VariantSpec {
        match self {
            Self::CoreHw1_0Itrng => &VariantSpec {
                variant: Self::CoreHw1_0Itrng,
                release: Release::V1x,
                subtarget: Subtarget::V1x,
                board: Board::Zcu104,
                hw_major_version: "1.x",
                fpga_dir: "hw/1.x/fpga",
                submodule_path: "hw/1.x/1.0/rtl",
                has_ss_commit: false,
                segmented: false,
                publish: true,
                timeout_minutes: 180,
                tclargs: &["BUILD=TRUE", "ITRNG=TRUE", "RTL_VERSION=1.0"],
                outputs: &["caliptra_fpga.bin"],
            },
            Self::CoreHw1_0Etrng => &VariantSpec {
                variant: Self::CoreHw1_0Etrng,
                release: Release::V1x,
                subtarget: Subtarget::V1x,
                board: Board::Zcu104,
                hw_major_version: "1.x",
                fpga_dir: "hw/1.x/fpga",
                submodule_path: "hw/1.x/1.0/rtl",
                has_ss_commit: false,
                segmented: false,
                publish: true,
                timeout_minutes: 180,
                tclargs: &["BUILD=TRUE", "ITRNG=FALSE", "RTL_VERSION=1.0"],
                outputs: &["caliptra_fpga.bin"],
            },
            Self::CoreLatestItrng => &VariantSpec {
                variant: Self::CoreLatestItrng,
                release: Release::V1x,
                subtarget: Subtarget::V1x,
                board: Board::Zcu104,
                hw_major_version: "1.x",
                fpga_dir: "hw/1.x/fpga",
                submodule_path: "hw/1.x/latest/rtl",
                has_ss_commit: false,
                segmented: false,
                publish: true,
                timeout_minutes: 180,
                tclargs: &["BUILD=TRUE", "ITRNG=TRUE", "RTL_VERSION=latest"],
                outputs: &["caliptra_fpga.bin"],
            },
            Self::CoreLatestEtrng => &VariantSpec {
                variant: Self::CoreLatestEtrng,
                release: Release::V1x,
                subtarget: Subtarget::V1x,
                board: Board::Zcu104,
                hw_major_version: "1.x",
                fpga_dir: "hw/1.x/fpga",
                submodule_path: "hw/1.x/latest/rtl",
                has_ss_commit: false,
                segmented: false,
                publish: true,
                timeout_minutes: 180,
                tclargs: &["BUILD=TRUE", "ITRNG=FALSE", "RTL_VERSION=latest"],
                outputs: &["caliptra_fpga.bin"],
            },
            Self::Core2_0Segmented => &VariantSpec {
                variant: Self::Core2_0Segmented,
                release: Release::V2_0,
                subtarget: Subtarget::V2_0Core,
                board: Board::Vck190,
                hw_major_version: "2.0",
                fpga_dir: "hw/2.0/core/fpga",
                submodule_path: "hw/2.0/caliptra-ss",
                has_ss_commit: true,
                segmented: true,
                publish: true,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=true"],
                outputs: &["caliptra_fpga.xsa", "runtime_<ver>.pdi"],
            },
            Self::Core2_0Standard => &VariantSpec {
                variant: Self::Core2_0Standard,
                release: Release::V2_0,
                subtarget: Subtarget::V2_0Core,
                board: Board::Vck190,
                hw_major_version: "2.0",
                fpga_dir: "hw/2.0/core/fpga",
                submodule_path: "hw/2.0/caliptra-ss",
                has_ss_commit: true,
                segmented: false,
                publish: false,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=false"],
                outputs: &["caliptra_fpga.xsa"],
            },
            Self::Subsystem2_0Segmented => &VariantSpec {
                variant: Self::Subsystem2_0Segmented,
                release: Release::V2_0,
                subtarget: Subtarget::V2_0Subsystem,
                board: Board::Vck190,
                hw_major_version: "2.0",
                fpga_dir: "hw/2.0/subsystem/fpga",
                submodule_path: "hw/2.0/caliptra-ss",
                has_ss_commit: true,
                segmented: true,
                publish: true,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=true"],
                outputs: &["xsa_ss2p0_<ver>_segmented.xsa", "runtime_<ver>.pdi"],
            },
            Self::Subsystem2_0Standard => &VariantSpec {
                variant: Self::Subsystem2_0Standard,
                release: Release::V2_0,
                subtarget: Subtarget::V2_0Subsystem,
                board: Board::Vck190,
                hw_major_version: "2.0",
                fpga_dir: "hw/2.0/subsystem/fpga",
                submodule_path: "hw/2.0/caliptra-ss",
                has_ss_commit: true,
                segmented: false,
                publish: false,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=false"],
                outputs: &["xsa_ss2p0_<ver>.xsa"],
            },
            Self::Core2_1Segmented => &VariantSpec {
                variant: Self::Core2_1Segmented,
                release: Release::V2_1,
                subtarget: Subtarget::V2_1Core,
                board: Board::Vck190,
                hw_major_version: "2.1",
                fpga_dir: "hw/2.1/core/fpga",
                submodule_path: "hw/2.1/caliptra-ss",
                has_ss_commit: true,
                segmented: true,
                publish: true,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=true"],
                outputs: &["caliptra_fpga.xsa", "runtime_<ver>.pdi"],
            },
            Self::Core2_1Standard => &VariantSpec {
                variant: Self::Core2_1Standard,
                release: Release::V2_1,
                subtarget: Subtarget::V2_1Core,
                board: Board::Vck190,
                hw_major_version: "2.1",
                fpga_dir: "hw/2.1/core/fpga",
                submodule_path: "hw/2.1/caliptra-ss",
                has_ss_commit: true,
                segmented: false,
                publish: false,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=false"],
                outputs: &["caliptra_fpga.xsa"],
            },
            Self::Subsystem2_1Segmented => &VariantSpec {
                variant: Self::Subsystem2_1Segmented,
                release: Release::V2_1,
                subtarget: Subtarget::V2_1Subsystem,
                board: Board::Vck190,
                hw_major_version: "2.1",
                fpga_dir: "hw/2.1/subsystem/fpga",
                submodule_path: "hw/2.1/caliptra-ss",
                has_ss_commit: true,
                segmented: true,
                publish: true,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=true"],
                outputs: &["xsa_ss2p1_<ver>_segmented.xsa", "runtime_<ver>.pdi"],
            },
            Self::Subsystem2_1Standard => &VariantSpec {
                variant: Self::Subsystem2_1Standard,
                release: Release::V2_1,
                subtarget: Subtarget::V2_1Subsystem,
                board: Board::Vck190,
                hw_major_version: "2.1",
                fpga_dir: "hw/2.1/subsystem/fpga",
                submodule_path: "hw/2.1/caliptra-ss",
                has_ss_commit: true,
                segmented: false,
                publish: false,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=false"],
                outputs: &["xsa_ss2p1_<ver>.xsa"],
            },
            Self::Core2_2Segmented => &VariantSpec {
                variant: Self::Core2_2Segmented,
                release: Release::V2_2,
                subtarget: Subtarget::V2_2Core,
                board: Board::Vck190,
                hw_major_version: "2.2",
                fpga_dir: "hw/2.2/core/fpga",
                submodule_path: "hw/2.2/caliptra-ss",
                has_ss_commit: true,
                segmented: true,
                publish: true,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=true"],
                outputs: &["caliptra_fpga.xsa", "runtime_<ver>.pdi"],
            },
            Self::Core2_2Standard => &VariantSpec {
                variant: Self::Core2_2Standard,
                release: Release::V2_2,
                subtarget: Subtarget::V2_2Core,
                board: Board::Vck190,
                hw_major_version: "2.2",
                fpga_dir: "hw/2.2/core/fpga",
                submodule_path: "hw/2.2/caliptra-ss",
                has_ss_commit: true,
                segmented: false,
                publish: false,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=false"],
                outputs: &["caliptra_fpga.xsa"],
            },
            Self::Subsystem2_2Segmented => &VariantSpec {
                variant: Self::Subsystem2_2Segmented,
                release: Release::V2_2,
                subtarget: Subtarget::V2_2Subsystem,
                board: Board::Vck190,
                hw_major_version: "2.2",
                fpga_dir: "hw/2.2/subsystem/fpga",
                submodule_path: "hw/2.2/caliptra-ss",
                has_ss_commit: true,
                segmented: true,
                publish: true,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=true"],
                outputs: &["xsa_ss2p2_<ver>_segmented.xsa", "runtime_<ver>.pdi"],
            },
            Self::Subsystem2_2Standard => &VariantSpec {
                variant: Self::Subsystem2_2Standard,
                release: Release::V2_2,
                subtarget: Subtarget::V2_2Subsystem,
                board: Board::Vck190,
                hw_major_version: "2.2",
                fpga_dir: "hw/2.2/subsystem/fpga",
                submodule_path: "hw/2.2/caliptra-ss",
                has_ss_commit: true,
                segmented: false,
                publish: false,
                timeout_minutes: 1440,
                tclargs: &["BUILD=TRUE", "SEGMENTED=false"],
                outputs: &["xsa_ss2p2_<ver>.xsa"],
            },
        }
    }
}

impl fmt::Display for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Variant {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|v| v.as_str() == s)
            .with_context(|| format!("unknown variant '{s}'"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildSelector {
    All,
    Release(Release),
    Subtarget(Subtarget),
    Variant(Variant),
}

impl fmt::Display for BuildSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => f.write_str("all"),
            Self::Release(r) => r.fmt(f),
            Self::Subtarget(st) => st.fmt(f),
            Self::Variant(v) => v.fmt(f),
        }
    }
}

impl FromStr for BuildSelector {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        if s == "all" {
            return Ok(Self::All);
        }
        if let Ok(r) = s.parse::<Release>() {
            return Ok(Self::Release(r));
        }
        if let Ok(st) = s.parse::<Subtarget>() {
            return Ok(Self::Subtarget(st));
        }
        if let Ok(v) = s.parse::<Variant>() {
            return Ok(Self::Variant(v));
        }
        bail!("unknown variant, release, or subtarget '{s}'. Run 'list' to see valid options.")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VariantSpec {
    pub variant: Variant,
    pub release: Release,
    pub subtarget: Subtarget,
    pub board: Board,
    pub hw_major_version: &'static str,
    pub fpga_dir: &'static str,
    pub submodule_path: &'static str,
    pub has_ss_commit: bool,
    pub segmented: bool,
    pub publish: bool,
    pub timeout_minutes: u32,
    pub tclargs: &'static [&'static str],
    pub outputs: &'static [&'static str],
}

impl VariantSpec {
    pub fn to_matrix_entry(&self) -> MatrixEntry {
        MatrixEntry {
            variant: self.variant,
            release: self.release,
            subtarget: self.subtarget,
            board: self.board,
            hw_major_version: self.hw_major_version.to_string(),
            submodule_path: self.submodule_path.to_string(),
            has_ss_commit: self.has_ss_commit,
            segmented: self.segmented,
            publish: self.publish,
            timeout_minutes: self.timeout_minutes,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatrixEntry {
    pub variant: Variant,
    pub release: Release,
    pub subtarget: Subtarget,
    pub board: Board,
    pub hw_major_version: String,
    pub submodule_path: String,
    pub has_ss_commit: bool,
    pub segmented: bool,
    pub publish: bool,
    pub timeout_minutes: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatrixOutput {
    pub include: Vec<MatrixEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushSelection {
    All,
    Subtargets(BTreeSet<Subtarget>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleArtifacts {
    Zcu104 {
        bin_file: PathBuf,
    },
    Vck190 {
        xsa_file: PathBuf,
        pdi_file: Option<PathBuf>,
    },
}

#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub selector: BuildSelector,
    pub dry_run: bool,
    pub staging_dir: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct BundleOptions {
    pub variant: Variant,
    pub staging_dir: PathBuf,
    pub output_dir: PathBuf,
    pub repository: String,
    pub target_branch: String,
    pub commit_hash: String,
    pub job_id: String,
}

pub fn all_specs() -> impl Iterator<Item = &'static VariantSpec> {
    Variant::ALL.iter().map(|v| v.spec())
}

pub fn select_variants(selector: BuildSelector) -> Vec<&'static VariantSpec> {
    all_specs()
        .filter(|spec| match selector {
            BuildSelector::All => true,
            BuildSelector::Release(r) => spec.release == r,
            BuildSelector::Subtarget(st) => spec.subtarget == st,
            BuildSelector::Variant(v) => spec.variant == v,
        })
        .collect()
}

pub fn resolve_repo_root(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(root) = explicit {
        return Ok(root.to_path_buf());
    }
    if let Ok(cwd) = env::current_dir() {
        if let Some(found) = walk_up_for_repo_root(&cwd) {
            return Ok(found);
        }
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            if let Some(found) = walk_up_for_repo_root(parent) {
                return Ok(found);
            }
        }
    }
    env::current_dir().context("failed to determine current working directory")
}

fn walk_up_for_repo_root(start: &Path) -> Option<PathBuf> {
    let mut cur = Some(start);
    while let Some(dir) = cur {
        if dir.join(".gitmodules").is_file() && dir.join("hw").is_dir() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

pub fn format_variants_table() -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<25} {:<7} {:<14} {:<7} {:<23} {:<31} {}\n",
        "VARIANT", "RELEASE", "SUBTARGET", "BOARD", "FPGA_DIR", "SUBMODULE_PATH", "TCLARGS / OUTPUTS"
    ));
    out.push_str(&format!(
        "{:<25} {:<7} {:<14} {:<7} {:<23} {:<31} {}\n",
        "-------", "-------", "---------", "-----", "--------", "--------------", "-----------------"
    ));
    for v in all_specs() {
        out.push_str(&format!(
            "{:<25} {:<7} {:<14} {:<7} {:<23} {:<31} {} -> [{}]\n",
            v.variant.as_str(),
            v.release.as_str(),
            v.subtarget.as_str(),
            v.board.as_str(),
            v.fpga_dir,
            v.submodule_path,
            v.tclargs.join(" "),
            v.outputs.join(", "),
        ));
    }
    out.push('\n');
    let releases: Vec<&str> = Release::ALL.iter().map(Release::as_str).collect();
    let subtargets: Vec<&str> = Subtarget::ALL.iter().map(Subtarget::as_str).collect();
    out.push_str(&format!("Releases:   {}\n", releases.join(" ")));
    out.push_str(&format!("Subtargets: {}\n", subtargets.join(" ")));
    out.push_str("All:        all\n");
    out
}

pub fn filter_dispatch_matrix(
    target: Option<&str>,
    variant: Option<&str>,
    include_standard: bool,
) -> Result<Vec<MatrixEntry>> {
    let mut entries: Vec<MatrixEntry> = if let Some(v) = variant.filter(|s| !s.is_empty()) {
        let parsed = v.parse::<Variant>()?;
        vec![parsed.spec().to_matrix_entry()]
    } else if let Some(t) = target.filter(|s| !s.is_empty() && *s != "all") {
        let selector = match BuildSelector::from_str(t) {
            Ok(BuildSelector::Release(r)) => BuildSelector::Release(r),
            Ok(BuildSelector::Subtarget(st)) => BuildSelector::Subtarget(st),
            _ => bail!("unknown target '{t}'"),
        };
        select_variants(selector)
            .into_iter()
            .map(VariantSpec::to_matrix_entry)
            .collect()
    } else {
        all_specs().map(VariantSpec::to_matrix_entry).collect()
    };

    if !include_standard {
        entries.retain(|e| !(e.board == Board::Vck190 && !e.segmented));
    }
    Ok(entries)
}

fn insert_matching_subtargets(path: &str, selected: &mut BTreeSet<Subtarget>) {
    // Prefer granular per-subtarget prefixes (`hw/2.x/core/...`, `hw/2.x/subsystem/...`)
    // before falling back to shared release directories (`hw/2.x/caliptra-ss`).
    if let Some(st) = Subtarget::ALL
        .iter()
        .copied()
        .find(|st| path.starts_with(st.hw_prefix()))
    {
        selected.insert(st);
        return;
    }

    for &release in Release::ALL {
        let is_shared_ss = release.ss_submodule_path().is_some_and(|p| path == p);
        if is_shared_ss || path.starts_with(release.hw_prefix()) {
            selected.extend(
                Subtarget::ALL
                    .iter()
                    .copied()
                    .filter(|st| st.release() == release),
            );
            return;
        }
    }
}

pub fn classify_push_paths<'a>(
    changed_paths: impl IntoIterator<Item = &'a str>,
    hw_diff_paths: impl IntoIterator<Item = &'a str>,
) -> PushSelection {
    let mut gitmodules_changed = false;
    let mut selected = BTreeSet::new();

    for path in changed_paths {
        if path.is_empty() {
            continue;
        }
        if path == ".github/workflows/bitstream-build.yml"
            || path == "ci-tools/bitstream-downloader"
            || path.starts_with("ci-tools/bitstream-downloader/")
        {
            return PushSelection::All;
        }
        if path == ".gitmodules" {
            gitmodules_changed = true;
        }
        insert_matching_subtargets(path, &mut selected);
    }

    if gitmodules_changed {
        for path in hw_diff_paths {
            if path.is_empty() {
                continue;
            }
            insert_matching_subtargets(path, &mut selected);
        }
        if selected.is_empty() {
            return PushSelection::All;
        }
    }

    PushSelection::Subtargets(selected)
}

fn git_commit_exists(repo_root: &Path, rev: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["cat-file", "-e", &format!("{rev}^{{commit}}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

fn git_diff_names(
    repo_root: &Path,
    before: &str,
    after: &str,
    pathspec: Option<&str>,
) -> Result<Vec<String>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo_root)
        .args(["diff", "--name-only", before, after]);
    if let Some(p) = pathspec {
        cmd.arg("--").arg(p);
    }
    let out = cmd.output().context("failed to execute git diff")?;
    if !out.status.success() {
        bail!(
            "git diff --name-only {before} {after} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

pub fn compute_ci_matrix(repo_root: &Path) -> Result<MatrixOutput> {
    let event_name = env::var("EVENT_NAME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| env::var("GITHUB_EVENT_NAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "workflow_dispatch".to_string());

    if event_name == "workflow_dispatch" {
        let input_target = env::var("INPUT_TARGET").ok();
        let input_variant = env::var("INPUT_VARIANT").ok();
        let include_standard = env::var("INPUT_INCLUDE_STANDARD")
            .ok()
            .filter(|s| !s.is_empty())
            .as_deref()
            != Some("false");
        let include = filter_dispatch_matrix(
            input_target.as_deref(),
            input_variant.as_deref(),
            include_standard,
        )?;
        return Ok(MatrixOutput { include });
    }

    let before_sha = env::var("BEFORE_SHA").unwrap_or_default();
    let after_sha = env::var("AFTER_SHA")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| env::var("GITHUB_SHA").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "HEAD".to_string());

    let selection = if before_sha.is_empty()
        || before_sha == "0000000000000000000000000000000000000000"
    {
        PushSelection::All
    } else {
        if !git_commit_exists(repo_root, &before_sha) {
            let _ = Command::new("git")
                .arg("-C")
                .arg(repo_root)
                .args(["fetch", "--no-tags", "--depth=1", "origin", &before_sha])
                .output();
        }
        if !git_commit_exists(repo_root, &before_sha) {
            PushSelection::All
        } else {
            let changed = git_diff_names(repo_root, &before_sha, &after_sha, None)?;
            let hw_diff = if changed.iter().any(|p| p == ".gitmodules") {
                git_diff_names(repo_root, &before_sha, &after_sha, Some("hw/"))?
            } else {
                Vec::new()
            };
            classify_push_paths(
                changed.iter().map(String::as_str),
                hw_diff.iter().map(String::as_str),
            )
        }
    };

    let include = match selection {
        PushSelection::All => all_specs().map(VariantSpec::to_matrix_entry).collect(),
        PushSelection::Subtargets(subtargets) => all_specs()
            .filter(|v| subtargets.contains(&v.subtarget))
            .map(VariantSpec::to_matrix_entry)
            .collect(),
    };
    Ok(MatrixOutput { include })
}

pub fn run_ci_matrix(repo_root: &Path) -> Result<()> {
    let output = compute_ci_matrix(repo_root)?;
    let matrix_json = serde_json::to_string(&output)?;
    let has_targets = !output.include.is_empty();
    let formatted = format!("matrix={matrix_json}\nhas_targets={has_targets}\n");

    if let Ok(dest) = env::var("GITHUB_OUTPUT") {
        if !dest.is_empty() {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&dest)
                .with_context(|| format!("failed to open GITHUB_OUTPUT '{dest}'"))?;
            file.write_all(formatted.as_bytes())?;
            return Ok(());
        }
    }
    print!("{formatted}");
    Ok(())
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    meta.is_file() && (meta.permissions().mode() & 0o111 != 0)
}

fn find_executable_on_path(bin: &str) -> bool {
    let bin_path = Path::new(bin);
    if bin_path.components().count() > 1 {
        return is_executable_file(bin_path);
    }
    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path_var).any(|dir| is_executable_file(&dir.join(bin)))
}

fn copy_built_artifacts(build_dir: &Path, staging_dir: &Path) -> Result<usize> {
    let mut entries: Vec<PathBuf> = fs::read_dir(build_dir)
        .with_context(|| format!("failed to read build directory {}", build_dir.display()))?
        .filter_map(|res| res.ok().map(|e| e.path()))
        .filter(|path| {
            path.is_file()
                && matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("bin" | "xsa" | "pdi")
                )
        })
        .collect();
    entries.sort();

    for src in &entries {
        let name = src.file_name().context("artifact missing filename")?;
        let dst = staging_dir.join(name);
        fs::copy(src, &dst).with_context(|| {
            format!("failed to copy {} to {}", src.display(), dst.display())
        })?;
    }
    Ok(entries.len())
}

pub fn run_build(repo_root: &Path, opts: &BuildOptions) -> Result<()> {
    let selected = select_variants(opts.selector);
    let vivado_bin = env::var("VIVADO_BIN")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_VIVADO_BIN.to_string());
    let vivado_settings = env::var("VIVADO_SETTINGS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_VIVADO_SETTINGS.to_string());

    let vivado_on_path = find_executable_on_path(&vivado_bin);
    let settings_exists = Path::new(&vivado_settings).is_file();

    if opts.dry_run {
        if !vivado_on_path {
            if !settings_exists {
                println!(
                    "[dry-run] # Note: {vivado_bin} not on PATH and {vivado_settings} not found locally"
                );
            }
            println!("[dry-run] source {vivado_settings}");
        }
    } else if !vivado_on_path && !settings_exists {
        bail!(
            "Vivado executable '{vivado_bin}' not found (checked PATH and '{vivado_settings}')."
        );
    }

    let staging_abs = if let Some(dir) = &opts.staging_dir {
        if opts.dry_run {
            if dir.is_absolute() {
                Some(dir.clone())
            } else {
                Some(env::current_dir()?.join(dir))
            }
        } else {
            fs::create_dir_all(dir).with_context(|| {
                format!("failed to create staging dir {}", dir.display())
            })?;
            Some(fs::canonicalize(dir).with_context(|| {
                format!("failed to canonicalize staging dir {}", dir.display())
            })?)
        }
    } else {
        None
    };

    for spec in selected {
        println!(
            "=== Building variant: {} (release={}, subtarget={}, board={}) ===",
            spec.variant, spec.release, spec.subtarget, spec.board
        );

        let submod_git = repo_root.join(spec.submodule_path).join(".git");
        if !submod_git.exists() {
            if opts.dry_run {
                println!(
                    "[dry-run] git -C {} submodule update --init --recursive {}",
                    repo_root.display(),
                    spec.submodule_path
                );
            } else {
                let status = Command::new("git")
                    .arg("-C")
                    .arg(repo_root)
                    .args([
                        "submodule",
                        "update",
                        "--init",
                        "--recursive",
                        spec.submodule_path,
                    ])
                    .status()
                    .with_context(|| {
                        format!("failed to initialize submodule {}", spec.submodule_path)
                    })?;
                if !status.success() {
                    bail!(
                        "git submodule update --init --recursive {} failed with status {status}",
                        spec.submodule_path
                    );
                }
            }
        }

        let fpga_dir = repo_root.join(spec.fpga_dir);
        if opts.dry_run {
            println!(
                "[dry-run] (cd {} && {} -mode batch -source fpga_configuration.tcl -tclargs {})",
                fpga_dir.display(),
                vivado_bin,
                spec.tclargs.join(" ")
            );
            if let Some(stage) = &staging_abs {
                println!("[dry-run] mkdir -p {}", stage.display());
                println!(
                    "[dry-run] cp {}/caliptra_build/*.{{bin,xsa,pdi}} {}/",
                    fpga_dir.display(),
                    stage.display()
                );
            }
            continue;
        }

        let status = if vivado_on_path {
            Command::new(&vivado_bin)
                .current_dir(&fpga_dir)
                .args([
                    "-mode",
                    "batch",
                    "-source",
                    "fpga_configuration.tcl",
                    "-tclargs",
                ])
                .args(spec.tclargs)
                .status()
                .with_context(|| format!("failed to invoke {vivado_bin}"))?
        } else {
            Command::new("bash")
                .current_dir(&fpga_dir)
                .args([
                    "-c",
                    "source \"$1\" >/dev/null && shift && exec \"$@\"",
                    "caliptra-bitstream-downloader",
                    &vivado_settings,
                    &vivado_bin,
                    "-mode",
                    "batch",
                    "-source",
                    "fpga_configuration.tcl",
                    "-tclargs",
                ])
                .args(spec.tclargs)
                .status()
                .with_context(|| {
                    format!("failed to invoke {vivado_bin} via {vivado_settings}")
                })?
        };
        if !status.success() {
            bail!(
                "Vivado build failed for {} with status {status}",
                spec.variant
            );
        }

        if let Some(stage) = &staging_abs {
            let build_dir = fpga_dir.join("caliptra_build");
            let copied = copy_built_artifacts(&build_dir, stage)?;
            if copied == 0 {
                bail!(
                    "no .bin/.xsa/.pdi artifacts found in {} for {}",
                    build_dir.display(),
                    spec.variant
                );
            }
        }
    }

    Ok(())
}

pub fn collect_bundle_artifacts(
    board: Board,
    segmented: bool,
    staging_dir: &Path,
) -> Result<BundleArtifacts> {
    if board == Board::Zcu104 {
        let bin_file = staging_dir.join("caliptra_fpga.bin");
        if !bin_file.is_file() {
            bail!(
                "expected {} from ZCU104 bitstream build",
                bin_file.display()
            );
        }
        return Ok(BundleArtifacts::Zcu104 { bin_file });
    }

    let mut xsa_files = Vec::new();
    let mut pdi_files = Vec::new();
    let entries = fs::read_dir(staging_dir)
        .with_context(|| format!("failed to read staging dir {}", staging_dir.display()))?;
    for entry in entries {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some("xsa") => xsa_files.push(path),
            Some("pdi") => pdi_files.push(path),
            _ => {}
        }
    }
    xsa_files.sort();
    pdi_files.sort();

    if xsa_files.len() != 1 {
        bail!(
            "expected exactly one .xsa file in {} (found {})",
            staging_dir.display(),
            xsa_files.len()
        );
    }
    if segmented && pdi_files.len() != 1 {
        bail!(
            "expected exactly one .pdi file for segmented build in {} (found {})",
            staging_dir.display(),
            pdi_files.len()
        );
    }
    if pdi_files.len() > 1 {
        bail!(
            "expected at most one .pdi file in {} (found {})",
            staging_dir.display(),
            pdi_files.len()
        );
    }

    Ok(BundleArtifacts::Vck190 {
        xsa_file: xsa_files.into_iter().next().unwrap(),
        pdi_file: pdi_files.into_iter().next(),
    })
}

pub fn resolve_submodule_commit(repo_root: &Path, submodule_path: &str) -> Result<String> {
    let submod_dir = repo_root.join(submodule_path);
    let out = if submod_dir.join(".git").exists() {
        Command::new("git")
            .arg("-C")
            .arg(&submod_dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .with_context(|| {
                format!(
                    "failed to run git rev-parse HEAD in {}",
                    submod_dir.display()
                )
            })?
    } else {
        Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(["rev-parse", &format!("HEAD:{submodule_path}")])
            .output()
            .with_context(|| {
                format!(
                    "failed to resolve gitlink HEAD:{submodule_path} in {}",
                    repo_root.display()
                )
            })?
    };

    if !out.status.success() {
        bail!(
            "failed to resolve submodule commit for {submodule_path}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn build_bundle_manifest(
    spec: &VariantSpec,
    opts: &BundleOptions,
    ss_commit: Option<String>,
) -> Manifest {
    Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
        repository: opts.repository.clone(),
        hw_major_version: spec.hw_major_version.to_string(),
        target_branch: opts.target_branch.clone(),
        caliptra_variant: spec.variant.as_str().to_string(),
        date: Utc::now().to_rfc3339(),
        commit_hash: opts.commit_hash.clone(),
        caliptra_ss_commit: ss_commit,
        job_id: opts.job_id.clone(),
        segmented: spec.segmented,
        github_pr: None,
        name: None,
        xsa_url: None,
        pdi_url: None,
        bin_url: None,
        xsa_hash: None,
        pdi_hash: None,
        bin_hash: None,
    }
}

pub fn format_step_summary(variant: Variant, commit_hash: &str) -> String {
    format!(
        "### Published Bitstream Manifest (`{variant}`)\n\n- **Manifest URL:** https://storage.googleapis.com/caliptra-github-ci-bitstreams/v1/{commit_hash}/{variant}/manifest.toml\n"
    )
}

pub async fn run_bundle(repo_root: &Path, opts: &BundleOptions) -> Result<()> {
    let spec = opts.variant.spec();
    let ss_commit = if spec.has_ss_commit {
        Some(resolve_submodule_commit(repo_root, spec.submodule_path)?)
    } else {
        None
    };
    let artifacts = collect_bundle_artifacts(spec.board, spec.segmented, &opts.staging_dir)?;

    fs::create_dir_all(&opts.output_dir).with_context(|| {
        format!(
            "failed to create output directory {}",
            opts.output_dir.display()
        )
    })?;

    let (xsa_path, pdi_path, bin_path) = match artifacts {
        BundleArtifacts::Zcu104 { bin_file } => (None, None, Some(bin_file)),
        BundleArtifacts::Vck190 { xsa_file, pdi_file } => (Some(xsa_file), pdi_file, None),
    };
    let manifest = build_bundle_manifest(spec, opts, ss_commit);

    crate::create_manifest_bundle(
        manifest,
        xsa_path,
        pdi_path,
        bin_path,
        opts.output_dir.clone(),
    )
    .await?;

    if spec.publish {
        if let Ok(summary_path) = env::var("GITHUB_STEP_SUMMARY") {
            if !summary_path.is_empty() {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&summary_path)
                    .with_context(|| {
                        format!("failed to open GITHUB_STEP_SUMMARY '{summary_path}'")
                    })?;
                file.write_all(format_step_summary(spec.variant, &opts.commit_hash).as_bytes())?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::collections::HashSet;
    use tar::Archive as TarArchive;
    use tempfile::TempDir;

    #[test]
    fn catalog_integrity() {
        assert_eq!(Variant::ALL.len(), 16);
        let mut names = HashSet::new();
        let mut releases = BTreeSet::new();
        let mut subtargets = BTreeSet::new();

        for &variant in Variant::ALL {
            let v = variant.spec();
            assert_eq!(v.variant, variant);
            assert_eq!(variant.as_str().parse::<Variant>().unwrap(), variant);
            assert!(names.insert(v.variant), "duplicate variant {}", v.variant);
            releases.insert(v.release);
            subtargets.insert(v.subtarget);
            assert!(!v.tclargs.is_empty());
            assert!(!v.outputs.is_empty());
        }

        let exp_releases: BTreeSet<Release> = Release::ALL.iter().copied().collect();
        let exp_subtargets: BTreeSet<Subtarget> = Subtarget::ALL.iter().copied().collect();
        assert_eq!(releases, exp_releases);
        assert_eq!(subtargets, exp_subtargets);
        assert_eq!(all_specs().filter(|v| v.publish).count(), 10);
    }

    #[test]
    fn select_variants_matching() {
        assert_eq!(select_variants(BuildSelector::All).len(), 16);
        assert_eq!(select_variants("all".parse().unwrap()).len(), 16);
        assert_eq!(select_variants(BuildSelector::Release(Release::V1x)).len(), 4);
        assert_eq!(select_variants(BuildSelector::Release(Release::V2_0)).len(), 4);
        assert_eq!(
            select_variants(BuildSelector::Subtarget(Subtarget::V2_1Core))
                .into_iter()
                .map(|v| v.variant)
                .collect::<Vec<_>>(),
            vec![Variant::Core2_1Segmented, Variant::Core2_1Standard]
        );
        assert_eq!(
            select_variants(BuildSelector::Variant(Variant::Subsystem2_2Segmented))
                .into_iter()
                .map(|v| v.variant)
                .collect::<Vec<_>>(),
            vec![Variant::Subsystem2_2Segmented]
        );
        assert!("nonexistent".parse::<BuildSelector>().is_err());
    }

    #[test]
    fn dispatch_matrix_filtering() {
        let all = filter_dispatch_matrix(Some("all"), None, true).unwrap();
        assert_eq!(all.len(), 16);
        assert_eq!(all.iter().filter(|e| e.publish).count(), 10);

        let no_std = filter_dispatch_matrix(Some("all"), None, false).unwrap();
        assert_eq!(no_std.len(), 10);
        assert!(no_std.iter().all(|e| e.board == Board::Zcu104 || e.segmented));

        let ss22 = filter_dispatch_matrix(Some("2.2/subsystem"), None, false).unwrap();
        assert_eq!(
            ss22.iter().map(|e| e.variant).collect::<Vec<_>>(),
            vec![Variant::Subsystem2_2Segmented]
        );
        let serialized = serde_json::to_string(&ss22[0]).unwrap();
        assert!(serialized.contains("\"variant\":\"subsystem-2.2-segmented\""));
        assert!(serialized.contains("\"release\":\"2.2\""));
        assert!(serialized.contains("\"subtarget\":\"2.2/subsystem\""));
        assert!(serialized.contains("\"board\":\"vck190\""));

        let v20 = filter_dispatch_matrix(Some("2.0"), None, true).unwrap();
        assert_eq!(v20.len(), 4);

        let override_var =
            filter_dispatch_matrix(Some("2.0"), Some("core-2.2-segmented"), true).unwrap();
        assert_eq!(
            override_var.iter().map(|e| e.variant).collect::<Vec<_>>(),
            vec![Variant::Core2_2Segmented]
        );

        assert!(filter_dispatch_matrix(Some("unknown"), None, true).is_err());
        assert!(filter_dispatch_matrix(Some("core-2.0-segmented"), None, true).is_err());
        assert!(filter_dispatch_matrix(None, Some("unknown"), true).is_err());
    }

    #[test]
    fn push_path_classification() {
        assert_eq!(
            classify_push_paths(["hw/2.2/subsystem/fpga/fpga_configuration.tcl"], []),
            PushSelection::Subtargets(BTreeSet::from([Subtarget::V2_2Subsystem]))
        );
        assert_eq!(
            classify_push_paths(["hw/2.2/core/fpga/src/caliptra_wrapper_top.sv"], []),
            PushSelection::Subtargets(BTreeSet::from([Subtarget::V2_2Core]))
        );
        assert_eq!(
            classify_push_paths(["hw/2.0/caliptra-ss"], []),
            PushSelection::Subtargets(BTreeSet::from([
                Subtarget::V2_0Core,
                Subtarget::V2_0Subsystem
            ]))
        );
        assert_eq!(
            classify_push_paths(["hw/2.1/caliptra-ss"], []),
            PushSelection::Subtargets(BTreeSet::from([
                Subtarget::V2_1Core,
                Subtarget::V2_1Subsystem
            ]))
        );
        assert_eq!(
            classify_push_paths(["hw/2.2/caliptra-ss"], []),
            PushSelection::Subtargets(BTreeSet::from([
                Subtarget::V2_2Core,
                Subtarget::V2_2Subsystem
            ]))
        );
        assert_eq!(
            classify_push_paths(["hw/1.x/fpga/src/caliptra_wrapper_top.sv"], []),
            PushSelection::Subtargets(BTreeSet::from([Subtarget::V1x]))
        );
        assert_eq!(
            classify_push_paths(["hw/fpga/create_boot_bin.sh", "hw/README.md"], []),
            PushSelection::Subtargets(BTreeSet::new())
        );
        assert_eq!(
            classify_push_paths(["ci-tools/bitstream-downloader/src/builder.rs"], []),
            PushSelection::All
        );
        assert_eq!(
            classify_push_paths([".github/workflows/bitstream-build.yml"], []),
            PushSelection::All
        );
        assert_eq!(
            classify_push_paths(
                [".gitmodules", "hw/2.1/caliptra-ss"],
                ["hw/2.1/caliptra-ss"]
            ),
            PushSelection::Subtargets(BTreeSet::from([
                Subtarget::V2_1Core,
                Subtarget::V2_1Subsystem
            ]))
        );
        assert_eq!(
            classify_push_paths([".gitmodules"], []),
            PushSelection::All
        );
    }

    #[tokio::test]
    async fn bundle_artifacts_and_in_process_bundle() {
        let tmp = TempDir::new().unwrap();

        // ZCU104 validation
        let zcu_dir = tmp.path().join("zcu");
        fs::create_dir_all(&zcu_dir).unwrap();
        assert!(collect_bundle_artifacts(Board::Zcu104, false, &zcu_dir).is_err());
        let bin_path = zcu_dir.join("caliptra_fpga.bin");
        fs::write(&bin_path, b"bin").unwrap();
        assert_eq!(
            collect_bundle_artifacts(Board::Zcu104, false, &zcu_dir).unwrap(),
            BundleArtifacts::Zcu104 {
                bin_file: bin_path.clone()
            }
        );

        // VCK190 segmented validation
        let vck_dir = tmp.path().join("vck");
        fs::create_dir_all(&vck_dir).unwrap();
        assert!(collect_bundle_artifacts(Board::Vck190, true, &vck_dir).is_err());
        let xsa_path = vck_dir.join("xsa_ss2p2_1234_segmented.xsa");
        fs::write(&xsa_path, b"xsa").unwrap();
        assert!(collect_bundle_artifacts(Board::Vck190, true, &vck_dir).is_err());
        let pdi_path = vck_dir.join("runtime_1234.pdi");
        fs::write(&pdi_path, b"pdi").unwrap();
        let artifacts = collect_bundle_artifacts(Board::Vck190, true, &vck_dir).unwrap();
        assert_eq!(
            artifacts,
            BundleArtifacts::Vck190 {
                xsa_file: xsa_path.clone(),
                pdi_file: Some(pdi_path.clone()),
            }
        );

        // Non-segmented VCK190 without PDI
        let std_dir = tmp.path().join("std");
        fs::create_dir_all(&std_dir).unwrap();
        let std_xsa = std_dir.join("caliptra_fpga.xsa");
        fs::write(&std_xsa, b"xsa").unwrap();
        assert_eq!(
            collect_bundle_artifacts(Board::Vck190, false, &std_dir).unwrap(),
            BundleArtifacts::Vck190 {
                xsa_file: std_xsa,
                pdi_file: None
            }
        );

        // End-to-end in-process bundle call for 1.x (no caliptra-ss commit recorded)
        let zcu_out_dir = tmp.path().join("out-zcu");
        let zcu_opts = BundleOptions {
            variant: Variant::CoreLatestItrng,
            staging_dir: zcu_dir,
            output_dir: zcu_out_dir.clone(),
            repository: "chipsalliance/caliptra-infra".to_string(),
            target_branch: "main".to_string(),
            commit_hash: "0123456789abcdef".to_string(),
            job_id: "98-build-bitstream".to_string(),
        };
        run_bundle(tmp.path(), &zcu_opts).await.unwrap();
        let zcu_extract = tmp.path().join("extract-zcu");
        TarArchive::new(GzDecoder::new(
            fs::File::open(zcu_out_dir.join(crate::OUTPUT_BUNDLE_FILENAME)).unwrap(),
        ))
        .unpack(&zcu_extract)
        .unwrap();
        let zcu_manifest = Manifest::load_from_path(&zcu_extract.join("manifest.toml"))
            .await
            .unwrap();
        assert_eq!(zcu_manifest.caliptra_variant, "core-latest-itrng");
        assert_eq!(zcu_manifest.caliptra_ss_commit, None);
        assert!(zcu_manifest.bin_hash.is_some());

        // End-to-end in-process bundle call for core-2.0-segmented (resolves hw/2.0/caliptra-ss)
        let ss20_dir = tmp.path().join("hw/2.0/caliptra-ss");
        fs::create_dir_all(&ss20_dir).unwrap();
        let run_git = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(&ss20_dir)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        run_git(&["init"]);
        run_git(&["config", "user.email", "test@example.com"]);
        run_git(&["config", "user.name", "Test"]);
        fs::write(ss20_dir.join("README"), b"ss").unwrap();
        run_git(&["add", "README"]);
        run_git(&["commit", "-m", "init"]);
        let expected_ss_commit = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&ss20_dir)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        let out_dir = tmp.path().join("out");
        let opts = BundleOptions {
            variant: Variant::Core2_0Segmented,
            staging_dir: vck_dir,
            output_dir: out_dir.clone(),
            repository: "chipsalliance/caliptra-infra".to_string(),
            target_branch: "main".to_string(),
            commit_hash: "0123456789abcdef".to_string(),
            job_id: "99-build-bitstream".to_string(),
        };
        run_bundle(tmp.path(), &opts).await.unwrap();

        let bundle_tar = out_dir.join(crate::OUTPUT_BUNDLE_FILENAME);
        assert!(bundle_tar.exists());
        let extract_dir = tmp.path().join("extract");
        let tar_gz = fs::File::open(&bundle_tar).unwrap();
        TarArchive::new(GzDecoder::new(tar_gz))
            .unpack(&extract_dir)
            .unwrap();
        let manifest = Manifest::load_from_path(&extract_dir.join("manifest.toml"))
            .await
            .unwrap();
        assert_eq!(manifest.caliptra_variant, "core-2.0-segmented");
        assert_eq!(manifest.hw_major_version, "2.0");
        assert_eq!(manifest.caliptra_ss_commit.as_deref(), Some(expected_ss_commit.as_str()));
        assert!(manifest.segmented);
        assert!(manifest.xsa_hash.is_some());
        assert!(manifest.pdi_hash.is_some());

        let summary = format_step_summary(Variant::Core2_0Segmented, &opts.commit_hash);
        assert!(summary.contains(
            "https://storage.googleapis.com/caliptra-github-ci-bitstreams/v1/0123456789abcdef/core-2.0-segmented/manifest.toml"
        ));
    }
}
