//! On-target tests for the flash-backed NVM store, no radio traffic involved.
//!
//! Verifies the FlashStore against real STM32WL flash: erased-region behavior,
//! save/load roundtrips of the actual persistence schema blobs, region independence,
//! and shrinking rewrites. Cross-reset persistence is exercised by the seed/check
//! pair: embedded-test restarts the firmware between tests, so `d_check_persisted`
//! reads what `c_seed_for_reset_check` wrote through a genuine MCU reset. The host
//! runs tests in alphabetical order, hence the a_/b_/c_/d_ prefixes: the wiping
//! tests must sort before the seed, and the seed before the check.
#![no_std]
#![no_main]

use defmt_rtt as _;

#[embedded_test::tests]
mod tests {
    use embassy_stm32::flash::Flash;
    use lora_hil_fw_tests::nvm::FlashStore;
    use lorawan_device::nvm::{NonVolatileStoreSync, NvmRegion, PersistentIdentity, MAX_BLOB_LEN};

    #[init]
    fn init() -> FlashStore<'static> {
        let p = embassy_stm32::init(embassy_stm32::Config::default());
        FlashStore::new(Flash::new_blocking(p.FLASH))
    }

    #[test]
    fn a_wiped_regions_load_none(mut store: FlashStore<'static>) {
        store.wipe().unwrap();
        let mut buf = [0u8; MAX_BLOB_LEN];
        assert!(store.load(NvmRegion::Identity, &mut buf).unwrap().is_none());
        assert!(store.load(NvmRegion::Session, &mut buf).unwrap().is_none());
    }

    #[test]
    fn b_roundtrip_and_region_independence(mut store: FlashStore<'static>) {
        store.wipe().unwrap();
        let identity =
            PersistentIdentity { dev_nonce: 41, last_join_nonce: Some(7), join_epoch: 3 };
        let mut blob = [0u8; MAX_BLOB_LEN];
        let n = identity.encode(&mut blob);
        store.save(NvmRegion::Identity, &blob[..n]).unwrap();

        let session_bytes = [0xA5u8; 64];
        store.save(NvmRegion::Session, &session_bytes).unwrap();

        let mut buf = [0u8; MAX_BLOB_LEN];
        let n = store.load(NvmRegion::Identity, &mut buf).unwrap().unwrap();
        let loaded = PersistentIdentity::decode(&buf[..n]).unwrap();
        assert_eq!(loaded.dev_nonce, 41);
        assert_eq!(loaded.last_join_nonce, Some(7));
        assert_eq!(loaded.join_epoch, 3);

        let n = store.load(NvmRegion::Session, &mut buf).unwrap().unwrap();
        assert_eq!(&buf[..n], &session_bytes);

        // Rewriting one region with a shorter blob must not disturb the other and
        // must not leak stale tail bytes past the new length.
        store.save(NvmRegion::Session, &[0x5A; 8]).unwrap();
        let n = store.load(NvmRegion::Session, &mut buf).unwrap().unwrap();
        assert_eq!(&buf[..n], &[0x5A; 8]);
        let n = store.load(NvmRegion::Identity, &mut buf).unwrap().unwrap();
        assert_eq!(PersistentIdentity::decode(&buf[..n]).unwrap().dev_nonce, 41);
    }

    #[test]
    fn c_seed_for_reset_check(mut store: FlashStore<'static>) {
        store.wipe().unwrap();
        let identity =
            PersistentIdentity { dev_nonce: 1234, last_join_nonce: Some(99), join_epoch: 12 };
        let mut blob = [0u8; MAX_BLOB_LEN];
        let n = identity.encode(&mut blob);
        store.save(NvmRegion::Identity, &blob[..n]).unwrap();
    }

    #[test]
    fn d_check_persisted(mut store: FlashStore<'static>) {
        // Runs in a fresh firmware execution after c_seed_for_reset_check: the
        // blob crossed an MCU reset in real flash.
        let mut buf = [0u8; MAX_BLOB_LEN];
        let n = store
            .load(NvmRegion::Identity, &mut buf)
            .unwrap()
            .expect("identity must survive reset");
        let identity = PersistentIdentity::decode(&buf[..n]).unwrap();
        assert_eq!(identity.dev_nonce, 1234);
        assert_eq!(identity.last_join_nonce, Some(99));
        assert_eq!(identity.join_epoch, 12);
    }
}
