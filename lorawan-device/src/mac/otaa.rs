use super::{del_to_delay_ms, session::Session, Response};
use crate::nvm::PersistentIdentity;
use crate::radio::RadioBuffer;
use crate::region::Configuration;
use crate::{AppEui, AppKey, DevEui};
use lorawan::creator::JoinRequest;
use lorawan::default_crypto::DefaultCrypto;
use lorawan::parser::DecryptedJoinAcceptPayload;
use rand_core::RngCore;

pub(crate) type DevNonce = lorawan::parser::DevNonce;

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
        Self { dev_nonce: DevNonce::from_value(0), network_credentials }
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
        self.dev_nonce =
            DevNonce::from_value(dev_nonce.unwrap_or_else(|| rng.next_u32() as u16));
        buf.clear();
        let request = JoinRequest {
            join_eui: self.network_credentials.appeui.into(),
            dev_eui: self.network_credentials.deveui.into(),
            dev_nonce: self.dev_nonce,
        };
        let crypto = DefaultCrypto::new(self.network_credentials.appkey.inner());
        let len = request.build_into(buf.as_mut(), &crypto).unwrap().len();
        buf.set_pos(len);
        self.dev_nonce.value()
    }

    pub(crate) fn handle_rx<const N: usize>(
        &mut self,
        region: &mut Configuration,
        configuration: &mut super::Configuration,
        identity: Option<&mut PersistentIdentity>,
        rx: &mut RadioBuffer<N>,
    ) -> Option<Session> {
        if let Ok(decrypt) = DecryptedJoinAcceptPayload::check_mic_and_decrypt_in_place(
            rx.as_mut_for_read(),
            &DefaultCrypto::new(self.network_credentials.appkey.inner()),
        ) {
            if let Some(identity) = identity {
                // A JoinAccept whose JoinNonce equals the last accepted one is a
                // replay of that accept: its MIC does not cover our DevNonce, so it
                // validates in a later join window while key derivation silently
                // diverges from the network. 1.0.4 guarantees only that the server's
                // JoinNonce is non-repeating (monotonicity is a 1.1 contract), so a
                // fresh accept is never equal to the last but may well be lower;
                // equality is the strongest test that cannot reject a compliant
                // server. Rejected before any region or configuration state is
                // touched, for the same reason the MIC is checked first.
                let join_nonce = decrypt.join_nonce().value();
                if identity.last_join_nonce == Some(join_nonce) {
                    return None;
                }
                identity.last_join_nonce = Some(join_nonce);
                identity.join_epoch = identity.join_epoch.wrapping_add(1);
            }
            region.process_join_accept(decrypt.c_f_list().as_ref());
            configuration.rx1_delay = del_to_delay_ms(decrypt.rx_delay());
            let dl_settings = decrypt.dl_settings();
            if let Some(rx1_dr_offset) = region.rx1_dr_offset_validate(dl_settings.rx1_dr_offset())
            {
                configuration.rx1_dr_offset = rx1_dr_offset;
            }
            let rx2_data_rate = dl_settings.rx2_data_rate();
            if region.get_datarate(rx2_data_rate as u8).is_some() {
                configuration.rx2_data_rate = Some(rx2_data_rate);
            }
            return Some(Session::derive_new(&decrypt, self.dev_nonce, &self.network_credentials));
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
