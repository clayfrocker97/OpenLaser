// SPDX-License-Identifier: GPL-3.0-or-later

//! Where things live in the controller's register space.
//!
//! Addresses count 16-bit registers; word counts are 32-bit words. A bank of
//! 20 words therefore spans 40 addresses, which is why the five parameter
//! banks sit 40 apart. Every value here was recovered from the vendor host
//! software and confirmed against captured traffic.

/// A block of words the host reads in one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Block {
    /// First register of the block.
    pub address: u32,
    /// How many 32-bit words the block holds.
    pub words: u16,
}

/// Machine status: identity, inputs, outputs, alarm aggregates, FIFO state.
pub const STATUS: Block = Block { address: 1000, words: 36 };
/// Live feedback for the five axis records, ten words each.
pub const AXES: Block = Block { address: 2000, words: 50 };
/// The head controller: alarm word, reference state, active command, height.
pub const HEAD: Block = Block { address: 10_000, words: 18 };
/// System configuration: interpolation cycle, axis routing, coordinate divisor.
pub const SYSTEM: Block = Block { address: 50_000, words: 26 };
/// The five axis parameter banks as 20-word records; 14 words of each are used.
pub const PARAMETERS: Block = Block { address: 50_200, words: 100 };
/// The fast poll: the five axis records followed by the five 14-word configs.
pub const RUNTIME: Block = Block { address: 60_001, words: 120 };
/// The first two status words on their own: product id and program version.
pub const IDENTITY: Block = Block { address: 1000, words: 2 };
/// The FIFO's total capacity in words.
pub const FIFO_CAPACITY: Block = Block { address: 1032, words: 1 };
/// The output port the controller uses for CO2 PWM synchronisation.
pub const CO2_PWM_SYNC: Block = Block { address: 50_108, words: 1 };
/// Both PWM synchronization routes, written by machine initialization.
pub const PWM_SYNC: Block = Block { address: 50_106, words: 2 };
/// Alarm detail cache, group 1: read when status aggregate bit 5 is set.
pub const DETAIL_GROUP_1: Block = Block { address: 5000, words: 1 };
/// Alarm detail cache, group 8: read when status aggregate bits 17..21 are set.
pub const DETAIL_GROUP_8: Block = Block { address: 11_000, words: 41 };

/// Bus reset and parameter activation writes.
pub const BUS: u32 = 100;
/// The command register: motion, head, outputs, laser, alarm relief.
pub const COMMAND: u32 = 101;
/// The program register: buffered motion records for the FIFO.
pub const PROGRAM: u32 = 102;
/// FIFO control: clear, start, stop.
pub const FIFO: u32 = 103;

/// Number of meaningful words in one axis parameter bank.
pub const PARAMETER_BANK_WORDS: usize = 14;
/// Number of axis records the controller reports.
pub const AXIS_COUNT: usize = 5;

/// The parameter bank of axis `index` (0 to 4), as read back and written.
#[must_use]
pub const fn parameter_bank(index: u8) -> u32 {
    PARAMETERS.address + 40 * index as u32
}

/// The soft limit pair of axis `index`: lower then upper, words 1 and 2 of
/// its parameter bank.
#[must_use]
pub const fn axis_limits(index: u8) -> u32 {
    parameter_bank(index) + 2
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bank stride is 40 addresses because a 20-word bank is 40 registers;
    /// the limit pair starts one word into the bank.
    #[test]
    fn parameter_banks_are_forty_registers_apart() {
        assert_eq!(parameter_bank(0), 50_200);
        assert_eq!(parameter_bank(1), 50_240);
        assert_eq!(parameter_bank(4), 50_360);
        assert_eq!(axis_limits(0), 50_202);
        assert_eq!(axis_limits(1), 50_242);
    }

    /// The fast poll block is exactly the five axis records plus five configs.
    #[test]
    fn runtime_block_holds_records_and_configs() {
        let records = AXIS_COUNT * 10;
        let configs = AXIS_COUNT * PARAMETER_BANK_WORDS;
        assert_eq!(usize::from(RUNTIME.words), records + configs);
        assert_eq!(usize::from(AXES.words), records);
        assert_eq!(usize::from(PARAMETERS.words), AXIS_COUNT * 20);
    }
}
