use super::{del_to_delay_ms, session::Session, Response};
use crate::nvm::PersistentIdentity;
use crate::radio::RadioBuffer;
use crate::region::Configuration;
use crate::{AppEui, AppKey, DevEui};
use lorawan::default_crypto::DefaultFactory;
use lorawan::{
    creator::JoinRequestCreator,
    parser::{parse as lorawan_parse, *},
};
use rand_core::RngCore;

pub(crate) type DevNonce = lorawan::parser::DevNonce<[u8; 2]>;

pub(crate) struct Otaa {
    dev_nonce: DevNonce,
    network_credentials: NetworkCredentials,
}
#[cfg_attr(feature = "defmt-03", derive(defmt::Format))]
#[derive(Debug, Clone)]
pub struct NetworkCredentials {
    deveui: DevEui,
    appeui: AppEui,
    appkey: AppKey,
}

impl Otaa {
    pub fn new(network_credentials: NetworkCredentials) -> Self {
        Self { dev_nonce: DevNonce::from([0, 0]), network_credentials }
    }

    /// Prepare a join request to be sent. This populates the radio buffer with the request to be
    /// sent, and returns the radio config to use for transmitting.
    ///
    /// `dev_nonce` selects the LoRaWAN 1.0.4 monotonic counter value when persistence is
    /// available; `None` falls back to the 1.0.2 random draw.
    pub(crate) fn prepare_buffer<G: RngCore, const N: usize>(
        &mut self,
        rng: &mut G,
        dev_nonce: Option<u16>,
        buf: &mut RadioBuffer<N>,
    ) -> u16 {
        self.dev_nonce = DevNonce::from(dev_nonce.unwrap_or_else(|| rng.next_u32() as u16));
        buf.clear();
        let mut phy = JoinRequestCreator::new(buf.as_mut()).unwrap();
        phy.set_app_eui(self.network_credentials.appeui)
            .set_dev_eui(self.network_credentials.deveui)
            .set_dev_nonce(self.dev_nonce);
        let crypto_factory = DefaultFactory;
        let len = phy.build(&self.network_credentials.appkey, &crypto_factory).len();
        buf.set_pos(len);
        u16::from(self.dev_nonce)
    }

    pub(crate) fn handle_rx<const N: usize>(
        &mut self,
        region: &mut Configuration,
        configuration: &mut super::Configuration,
        identity: Option<&mut PersistentIdentity>,
        rx: &mut RadioBuffer<N>,
    ) -> Option<Session> {
        if let Ok(PhyPayload::JoinAccept(JoinAcceptPayload::Encrypted(encrypted))) =
            lorawan_parse(rx.as_mut_for_read())
        {
            let decrypt = encrypted.decrypt(&self.network_credentials.appkey, &DefaultFactory);
            region.process_join_accept(&decrypt);
            // TODO: dlsettings (rx1_dr_offset / rx2_datarate)
            configuration.rx1_delay = del_to_delay_ms(decrypt.rx_delay());
            if decrypt.validate_mic(&self.network_credentials.appkey, &DefaultFactory) {
                if let Some(identity) = identity {
                    // A JoinAccept whose JoinNonce equals the last accepted one is a
                    // replay of that accept: its MIC does not cover our DevNonce, so it
                    // validates in a later join window while key derivation silently
                    // diverges from the network. 1.0.4 guarantees only that the server's
                    // JoinNonce is non-repeating (monotonicity is a 1.1 contract), so a
                    // fresh accept is never equal to the last but may well be lower;
                    // equality is the strongest test that cannot reject a compliant
                    // server.
                    let n = decrypt.app_nonce();
                    let n = n.as_ref();
                    let join_nonce = u32::from_le_bytes([n[0], n[1], n[2], 0]);
                    if identity.last_join_nonce == Some(join_nonce) {
                        return None;
                    }
                    identity.last_join_nonce = Some(join_nonce);
                    identity.join_epoch = identity.join_epoch.wrapping_add(1);
                }
                return Some(Session::derive_new(
                    &decrypt,
                    self.dev_nonce,
                    &self.network_credentials,
                ));
            }
        }
        None
    }

    pub(crate) fn rx2_complete(&mut self) -> Response {
        Response::NoJoinAccept
    }
}

impl NetworkCredentials {
    pub fn new(appeui: AppEui, deveui: DevEui, appkey: AppKey) -> Self {
        Self { deveui, appeui, appkey }
    }
    pub fn appeui(&self) -> &AppEui {
        &self.appeui
    }

    pub fn deveui(&self) -> &DevEui {
        &self.deveui
    }

    pub fn appkey(&self) -> &AppKey {
        &self.appkey
    }
}
