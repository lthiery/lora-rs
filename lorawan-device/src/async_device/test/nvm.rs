//! Tests for the LoRaWAN 1.0.4 join rules: monotonic DevNonce with commit-before-transmit
//! and JoinNonce replay rejection. "Power loss" is simulated by dropping the device and
//! calling [`Device::restore`] against the same mock store.

use super::radio::TestRadio;
use super::timer::TestTimer;
use super::{JoinResponse, region};
#[cfg(feature = "certification")]
use crate::async_device::SendResponse;
use crate::nvm::{MAX_BLOB_LEN, NonVolatileStoreSync, NvmRegion, PersistentIdentity};
#[cfg(feature = "certification")]
use crate::radio::RfConfig;
#[cfg(feature = "certification")]
use crate::test_util::get_abp_credentials;
use crate::test_util::{Uplink, get_crypto, get_network_crypto, get_otaa_credentials};
#[cfg(feature = "certification")]
use core::num::NonZeroU8;
#[cfg(feature = "certification")]
use lora_modulation::BaseBandModulationParams;
use lorawan::creator::JoinAccept;
#[cfg(feature = "certification")]
use lorawan::creator::{DataFrame, Payload};
#[cfg(feature = "certification")]
use lorawan::parser::DataFrameType;
use lorawan::parser::{self, JoinNonce, NetId, PhyPayload};
use lorawan::types::DLSettings;
use std::convert::Infallible;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

type Device =
    crate::async_device::Device<TestRadio, TestTimer, rand_core::OsRng, 512, 4, MockStore>;

#[derive(Default)]
struct Inner {
    blob: Option<Vec<u8>>,
    saves: usize,
}

/// In-memory store implementing the *sync* trait; the device consumes it through the
/// blanket async impl, so these tests cover that path too.
#[derive(Clone, Default)]
struct MockStore {
    inner: Arc<Mutex<Inner>>,
}

impl MockStore {
    fn saves(&self) -> usize {
        self.inner.lock().unwrap().saves
    }

    fn identity(&self) -> PersistentIdentity {
        PersistentIdentity::decode(self.inner.lock().unwrap().blob.as_ref().unwrap()).unwrap()
    }

    fn put(&self, identity: &PersistentIdentity) {
        let mut buf = [0u8; MAX_BLOB_LEN];
        let n = identity.encode(&mut buf);
        self.inner.lock().unwrap().blob = Some(buf[..n].to_vec());
    }

    fn corrupt(&self) {
        let mut inner = self.inner.lock().unwrap();
        *inner.blob.as_mut().unwrap().last_mut().unwrap() ^= 0x01;
    }
}

impl NonVolatileStoreSync for MockStore {
    type Error = Infallible;

    fn save(&mut self, region: NvmRegion, bytes: &[u8]) -> Result<(), Infallible> {
        let NvmRegion::Identity = region;
        let mut inner = self.inner.lock().unwrap();
        inner.blob = Some(bytes.to_vec());
        inner.saves += 1;
        Ok(())
    }

    fn load(&mut self, region: NvmRegion, buf: &mut [u8]) -> Result<Option<usize>, Infallible> {
        let NvmRegion::Identity = region;
        Ok(self.inner.lock().unwrap().blob.as_ref().map(|blob| {
            buf[..blob.len()].copy_from_slice(blob);
            blob.len()
        }))
    }
}

async fn restore(
    store: &MockStore,
) -> (super::radio::RadioChannel, super::timer::TimerChannel, Device) {
    let pending = Arc::new(AtomicUsize::new(0));
    let (radio_channel, mock_radio) = TestRadio::new(pending.clone());
    let (timer_channel, mock_timer) = TestTimer::new(pending);
    let region = region::US915::default();
    let device =
        Device::restore(region.into(), mock_radio, mock_timer, rand_core::OsRng, store.clone())
            .await
            .unwrap();
    (radio_channel, timer_channel, device)
}

/// Network-side join handler: asserts the transmitted DevNonce equals `EXPECT_DEV_NONCE`
/// (monotonic counter) and answers with JoinNonce `NONCE`.
fn join_accept<const NONCE: u8, const EXPECT_DEV_NONCE: u16>(
    uplink: Option<Uplink>,
    _config: crate::radio::RfConfig,
    rx_buffer: &mut [u8],
) -> usize {
    let mut uplink = uplink.expect("no uplink");
    let Ok(PhyPayload::JoinRequest(join_request)) = parser::parse(uplink.data_mut()) else {
        panic!("not a join request");
    };
    assert!(join_request.validate_mic(&get_crypto()));
    assert_eq!(
        join_request.dev_nonce().value(),
        EXPECT_DEV_NONCE,
        "DevNonce not the expected counter value"
    );

    let accept = JoinAccept {
        join_nonce: JoinNonce::from_wire_bytes([NONCE, 0, 0]),
        net_id: NetId::from_wire_bytes([1; 3]),
        dev_addr: crate::test_util::get_dev_addr(),
        dl_settings: DLSettings::new(0),
        rx_delay: 0,
        c_f_list: None,
    };
    accept.build_into(rx_buffer, &get_network_crypto()).unwrap().len()
}

#[tokio::test]
async fn failed_join_still_burns_a_durable_dev_nonce() {
    let store = MockStore::default();
    let (radio, timer, mut device) = restore(&store).await;

    let task = tokio::spawn(async move { device.join(&get_otaa_credentials()).await });
    timer.fire_most_recent().await; // RX1 open
    radio.handle_timeout().await; // RX1 timeout
    timer.fire_most_recent().await; // RX2 open
    radio.handle_timeout().await; // RX2 timeout
    assert!(matches!(task.await.unwrap(), Ok(JoinResponse::NoJoinAccept)));

    // DevNonce 0 went on the air and its increment was committed, despite no accept.
    let identity = store.identity();
    assert_eq!(identity.dev_nonce, 1);
    assert_eq!(identity.last_join_nonce, None);
}

#[tokio::test]
async fn restore_with_seed_keeps_the_dev_nonce_counter() {
    type SeededDevice =
        crate::async_device::Device<TestRadio, TestTimer, crate::Prng, 512, 4, MockStore>;
    let store = MockStore::default();
    for expected in 1..=2 {
        let pending = Arc::new(AtomicUsize::new(0));
        let (radio_channel, mock_radio) = TestRadio::new(pending.clone());
        let (timer_channel, mock_timer) = TestTimer::new(pending);
        let region = region::US915::default();
        let mut device = SeededDevice::restore_with_seed(
            region.into(),
            mock_radio,
            mock_timer,
            0x5EED,
            store.clone(),
        )
        .await
        .unwrap();
        let task = tokio::spawn(async move { device.join(&get_otaa_credentials()).await });
        timer_channel.fire_most_recent().await;
        radio_channel.handle_timeout().await;
        timer_channel.fire_most_recent().await;
        radio_channel.handle_timeout().await;
        assert!(matches!(task.await.unwrap(), Ok(JoinResponse::NoJoinAccept)));
        assert_eq!(store.identity().dev_nonce, expected);
    }
}

#[tokio::test]
async fn dev_nonce_monotonic_across_power_loss() {
    let store = MockStore::default();

    // First boot, first join: DevNonce 0.
    let (radio, timer, mut device) = restore(&store).await;
    let task = tokio::spawn(async move { device.join(&get_otaa_credentials()).await });
    timer.fire_most_recent().await;
    radio.handle_rxtx(join_accept::<1, 0>).await;
    assert!(matches!(task.await.unwrap(), Ok(JoinResponse::JoinSuccess)));
    let identity = store.identity();
    assert_eq!((identity.dev_nonce, identity.last_join_nonce), (1, Some(1)));

    // Power loss. Next join must use DevNonce 1, not restart at 0 or draw randomly.
    let (radio, timer, mut device) = restore(&store).await;
    let task = tokio::spawn(async move { device.join(&get_otaa_credentials()).await });
    timer.fire_most_recent().await;
    radio.handle_rxtx(join_accept::<2, 1>).await;
    assert!(matches!(task.await.unwrap(), Ok(JoinResponse::JoinSuccess)));
    let identity = store.identity();
    assert_eq!((identity.dev_nonce, identity.last_join_nonce), (2, Some(2)));
}

#[tokio::test]
async fn stored_dev_nonce_drives_the_next_join() {
    let store = MockStore::default();
    store.put(&PersistentIdentity { dev_nonce: 7, last_join_nonce: Some(3), ..Default::default() });

    let (radio, timer, mut device) = restore(&store).await;
    let task = tokio::spawn(async move { device.join(&get_otaa_credentials()).await });
    timer.fire_most_recent().await;
    radio.handle_rxtx(join_accept::<4, 7>).await;
    assert!(matches!(task.await.unwrap(), Ok(JoinResponse::JoinSuccess)));
    assert_eq!(store.identity().dev_nonce, 8);
}

#[tokio::test]
async fn corrupt_identity_restarts_the_dev_nonce() {
    // A torn identity write is treated as absent: the counter restarts at 0 and a join
    // server will reject the device until it passes the last value it saw. This is why
    // stores must write the identity atomically.
    let store = MockStore::default();
    store.put(&PersistentIdentity { dev_nonce: 7, last_join_nonce: Some(3), ..Default::default() });
    store.corrupt();

    let (radio, timer, mut device) = restore(&store).await;
    let task = tokio::spawn(async move { device.join(&get_otaa_credentials()).await });
    timer.fire_most_recent().await;
    radio.handle_rxtx(join_accept::<4, 0>).await;
    assert!(matches!(task.await.unwrap(), Ok(JoinResponse::JoinSuccess)));
    assert_eq!(store.identity().dev_nonce, 1);
}

#[tokio::test]
async fn replayed_join_accept_rejected() {
    let store = MockStore::default();
    let (radio, timer, mut device) = restore(&store).await;

    // Legitimate join with JoinNonce 5.
    let task = tokio::spawn(async move {
        let r = device.join(&get_otaa_credentials()).await;
        (device, r)
    });
    timer.fire_most_recent().await;
    radio.handle_rxtx(join_accept::<5, 0>).await;
    let (mut device, response) = task.await.unwrap();
    assert!(matches!(response, Ok(JoinResponse::JoinSuccess)));

    // An attacker replays a captured accept (JoinNonce 5 again): must be ignored,
    // ending in NoJoinAccept after both windows.
    let task = tokio::spawn(async move {
        let r = device.join(&get_otaa_credentials()).await;
        (device, r)
    });
    timer.fire_most_recent().await; // RX1 open

    // The replayed nonce is ignored; give the device time to reject the frame and arm
    // the RX2 timer.
    radio.handle_rxtx(join_accept::<5, 1>).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(15)).await;
    timer.fire_most_recent().await; // RX2 open
    radio.handle_timeout().await; // RX2 timeout
    let (mut device, response) = task.await.unwrap();
    assert!(matches!(response, Ok(JoinResponse::NoJoinAccept)));
    let identity = store.identity();
    assert_eq!((identity.dev_nonce, identity.last_join_nonce), (2, Some(5)));

    // A fresh accept with a LOWER JoinNonce is accepted: 1.0.4 servers guarantee only
    // non-repetition, not monotonicity, so anything different from the last must pass.
    let task = tokio::spawn(async move {
        let r = device.join(&get_otaa_credentials()).await;
        (device, r)
    });
    timer.fire_most_recent().await;
    radio.handle_rxtx(join_accept::<3, 2>).await;
    let (mut device, response) = task.await.unwrap();
    assert!(matches!(response, Ok(JoinResponse::JoinSuccess)));
    assert_eq!(store.identity().last_join_nonce, Some(3));

    // The replay floor tracks only the latest accept: nonce 5 is from two joins ago,
    // no longer the last accepted value, so it passes now.
    let task = tokio::spawn(async move { device.join(&get_otaa_credentials()).await });
    timer.fire_most_recent().await;
    radio.handle_rxtx(join_accept::<5, 3>).await;
    assert!(matches!(task.await.unwrap(), Ok(JoinResponse::JoinSuccess)));
    assert_eq!(store.identity().last_join_nonce, Some(5));
}

#[tokio::test]
async fn exhausted_dev_nonce_refuses_to_join() {
    let store = MockStore::default();
    store.put(&PersistentIdentity {
        dev_nonce: u16::MAX,
        last_join_nonce: Some(3),
        ..Default::default()
    });

    let (_radio, _timer, mut device) = restore(&store).await;
    let response = device.join(&get_otaa_credentials()).await;
    assert!(matches!(
        response,
        Err(crate::async_device::Error::Mac(crate::mac::Error::DevNonceExhausted))
    ));
    // Nothing was written and nothing went on the air.
    assert_eq!(store.saves(), 0);
    assert_eq!(store.identity().dev_nonce, u16::MAX);
}

/// Build an FPort 224 downlink with `data` at FCntDown 1 for the ABP test session.
#[cfg(feature = "certification")]
fn certification_downlink(data: &[u8], buf: &mut [u8]) -> usize {
    let frame = DataFrame {
        frame_type: DataFrameType::UnconfirmedDown,
        dev_addr: crate::test_util::get_dev_addr(),
        fcnt: 1,
        payload: Payload::Data { f_port: NonZeroU8::new(224).unwrap(), data },
        ..Default::default()
    };
    frame.build_into(buf, &get_crypto(), Some(&get_crypto())).unwrap().len()
}

#[cfg(feature = "certification")]
#[tokio::test]
async fn closed_certification_port_survives_a_reboot() {
    let store = MockStore::default();
    let (radio, timer, mut device) = restore(&store).await;
    assert!(matches!(device.join(&get_abp_credentials()).await, Ok(JoinResponse::JoinSuccess)));
    assert!(device.certification_enabled());
    device.disable_certification().await.unwrap();
    assert!(!device.certification_enabled());
    assert!(store.identity().fport224_disabled);
    // Closing it again changes nothing, so nothing is written.
    let saves = store.saves();
    device.disable_certification().await.unwrap();
    assert_eq!(store.saves(), saves);
    drop((radio, timer, device));

    let (radio, timer, mut device) = restore(&store).await;
    assert!(!device.certification_enabled());
    assert!(matches!(device.join(&get_abp_credentials()).await, Ok(JoinResponse::JoinSuccess)));

    // A LinkCheckReq on FPort 224 is plain application data now and reaches the
    // application as a downlink.
    fn cp_link_check_req(_uplink: Option<Uplink>, _config: RfConfig, buf: &mut [u8]) -> usize {
        certification_downlink(&[0x20], buf)
    }
    let task = tokio::spawn(async move {
        let r = device.send(&[1, 2, 3], 3, false).await;
        (device, r)
    });
    timer.fire_most_recent().await;
    radio.handle_rxtx(cp_link_check_req).await;
    let (mut device, response) = task.await.unwrap();
    assert!(matches!(response, Ok(SendResponse::DownlinkReceived(1))));
    let downlink = device.take_downlink().expect("delivered to the application");
    assert_eq!(downlink.fport, 224);
    assert_eq!(downlink.data.as_slice(), &[0x20]);
}

/// Closing the port over the air reaches the application as a reset event only after
/// the identity is in the store. The async device has no application-side handler for
/// these events yet, so the response path is driven directly.
#[cfg(feature = "certification")]
#[tokio::test]
async fn port_disable_persists_before_the_reset_event() {
    use crate::mac::{DeviceEvent, Response};
    let store = MockStore::default();
    let (_radio, _timer, mut device) = restore(&store).await;
    assert!(matches!(device.join(&get_abp_credentials()).await, Ok(JoinResponse::JoinSuccess)));
    assert_eq!(store.saves(), 0);

    let mut buf = [0u8; 256];
    let len = certification_downlink(&[0x7E], &mut buf);
    device.radio_buffer.clear();
    device.radio_buffer.extend_from_slice(&buf[..len]).unwrap();
    let dr = device.mac.region.get_datarate(8).unwrap();
    let rf_config = RfConfig {
        frequency: 923_300_000,
        bb: BaseBandModulationParams::new(
            dr.spreading_factor,
            dr.bandwidth,
            device.mac.region.get_coding_rate(),
        ),
        max_payload_len: 255,
    };
    let response = device.mac.handle_rx::<512, 4>(
        &mut device.radio_buffer,
        &mut device.downlink,
        10,
        &rf_config,
    );
    assert!(matches!(response, Response::DeviceHandler(DeviceEvent::ResetDevice)));
    assert!(!device.certification_enabled());
    assert_eq!(store.saves(), 0, "not yet persisted");

    let passed_on = Device::handle_mac_response(
        &mut device.radio_buffer,
        &mut device.retransmit_buffer,
        &mut device.mac,
        &mut device.radio,
        &mut device.rng,
        &mut device.store,
        response,
        None,
    )
    .await
    .unwrap();
    assert!(matches!(passed_on, Some(Response::DeviceHandler(DeviceEvent::ResetDevice))));
    assert_eq!(store.saves(), 1);
    assert!(store.identity().fport224_disabled);
}
