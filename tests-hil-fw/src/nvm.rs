//! Flash-backed [`NonVolatileStoreSync`] for the NUCLEO-WL55JC DUT.
//!
//! Backs the lorawan-device persistence layer (LoRaWAN 1.0.4: monotonic DevNonce,
//! JoinNonce replay floor, session resume) with the STM32WL's internal flash. The top
//! two 2 KB pages of the 256 KB bank are reserved, one per [`NvmRegion`]:
//!
//! - Identity: 0x3F800..0x40000 (offsets relative to FLASH_BASE 0x0800_0000)
//! - Session:  0x3F000..0x3F800
//!
//! The test ELF is ~110 KB, so nothing links this high; probe-rs only erases the
//! sectors the ELF occupies, so these pages survive both resets and re-flashes. That
//! is what makes cross-reset persistence observable from the harness. The flip side:
//! scenarios needing virgin state must call [`FlashStore::wipe`] first, or a previous
//! run's counters leak in.
//!
//! Page layout: `[magic u32 | len u32 | blob, padded to the 8-byte write unit]`.
//! save() is erase-then-write, which is NOT power-loss atomic; the stack's blob CRC
//! detects a torn write and treats the region as absent, which is the documented
//! degraded mode and fine for bench use.

use embassy_stm32::flash::{Blocking, Error as FlashError, Flash, WRITE_SIZE};
use lorawan_device::nvm::{NonVolatileStoreSync, NvmRegion, MAX_BLOB_LEN};

const PAGE_SIZE: u32 = 2048;
const SESSION_OFFSET: u32 = 0x3F000;
const IDENTITY_OFFSET: u32 = 0x3F800;
const MAGIC: u32 = 0x4C4E_564D; // "LNVM"
const HEADER_LEN: usize = 8;

// Header plus a padded max blob must fit the page.
const _: () = assert!(HEADER_LEN + MAX_BLOB_LEN + WRITE_SIZE <= PAGE_SIZE as usize);

fn page(region: NvmRegion) -> u32 {
    match region {
        NvmRegion::Identity => IDENTITY_OFFSET,
        NvmRegion::Session => SESSION_OFFSET,
    }
}

pub struct FlashStore<'d> {
    flash: Flash<'d, Blocking>,
}

impl<'d> FlashStore<'d> {
    pub fn new(flash: Flash<'d, Blocking>) -> Self {
        Self { flash }
    }

    /// Erase both regions. Test scenarios that need first-boot behavior call this;
    /// nothing else does, so state persists across resets and re-flashes.
    pub fn wipe(&mut self) -> Result<(), FlashError> {
        for offset in [SESSION_OFFSET, IDENTITY_OFFSET] {
            self.flash.blocking_erase(offset, offset + PAGE_SIZE)?;
        }
        Ok(())
    }
}

impl NonVolatileStoreSync for FlashStore<'_> {
    type Error = FlashError;

    fn save(&mut self, region: NvmRegion, bytes: &[u8]) -> Result<(), FlashError> {
        let offset = page(region);
        self.flash.blocking_erase(offset, offset + PAGE_SIZE)?;

        let mut buf = [0xFFu8; HEADER_LEN + MAX_BLOB_LEN + WRITE_SIZE];
        buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        buf[4..8].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        buf[HEADER_LEN..HEADER_LEN + bytes.len()].copy_from_slice(bytes);
        let write_len = (HEADER_LEN + bytes.len()).next_multiple_of(WRITE_SIZE);
        self.flash.blocking_write(offset, &buf[..write_len])
    }

    fn load(&mut self, region: NvmRegion, buf: &mut [u8]) -> Result<Option<usize>, FlashError> {
        let offset = page(region);
        let mut header = [0u8; HEADER_LEN];
        self.flash.blocking_read(offset, &mut header)?;
        let magic = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        if magic != MAGIC {
            // Erased (0xFFFFFFFF) or unrecognized: never written.
            return Ok(None);
        }
        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        if len > buf.len() {
            // A length no caller would ask for back means a torn or foreign header;
            // absent is the contract for unreadable regions.
            return Ok(None);
        }
        // Reads only need byte alignment on this family, so read the exact span.
        self.flash.blocking_read(offset + HEADER_LEN as u32, &mut buf[..len])?;
        Ok(Some(len))
    }
}
