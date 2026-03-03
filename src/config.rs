/// Which columns are visible in the table view.
#[derive(Debug, Clone)]
pub struct ColumnVisibility {
    pub vms: bool,
    pub cpu: bool,
    pub mem: bool,
    pub net: bool,
    pub io: bool,
    pub time_up: bool,
    pub last_chg: bool,
}

impl Default for ColumnVisibility {
    fn default() -> Self {
        Self {
            vms: true,
            cpu: true,
            mem: true,
            net: false,
            io: false,
            time_up: true,
            last_chg: true,
        }
    }
}

impl ColumnVisibility {
    /// Toggle column by number key (1-7).
    pub fn toggle(&mut self, key: u8) {
        match key {
            1 => self.vms = !self.vms,
            2 => self.cpu = !self.cpu,
            3 => self.mem = !self.mem,
            4 => self.net = !self.net,
            5 => self.io = !self.io,
            6 => self.time_up = !self.time_up,
            7 => self.last_chg = !self.last_chg,
            _ => {}
        }
    }
}

/// Column to sort by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Vms,
    Cpu,
    Mem,
    NetRx,
    IoRead,
    TimeUp,
    LastChg,
}

impl SortColumn {
    pub const ALL: &'static [SortColumn] = &[
        SortColumn::Name,
        SortColumn::Vms,
        SortColumn::Cpu,
        SortColumn::Mem,
        SortColumn::NetRx,
        SortColumn::IoRead,
        SortColumn::TimeUp,
        SortColumn::LastChg,
    ];

    /// Next sort column (cycle forward).
    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&s| s == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// Previous sort column (cycle backward).
    pub fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|&s| s == self).unwrap_or(0);
        if idx == 0 {
            Self::ALL[Self::ALL.len() - 1]
        } else {
            Self::ALL[idx - 1]
        }
    }

    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            SortColumn::Name => "ENVIRONMENT",
            SortColumn::Vms => "VMS",
            SortColumn::Cpu => "CPU",
            SortColumn::Mem => "MEM",
            SortColumn::NetRx => "NET",
            SortColumn::IoRead => "IO",
            SortColumn::TimeUp => "TIME-UP",
            SortColumn::LastChg => "LAST-CHG",
        }
    }
}

/// Sort configuration.
#[derive(Debug, Clone)]
pub struct SortConfig {
    pub column: SortColumn,
    pub ascending: bool,
}

impl Default for SortConfig {
    fn default() -> Self {
        Self {
            column: SortColumn::Name,
            ascending: true,
        }
    }
}
