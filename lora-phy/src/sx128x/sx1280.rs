use crate::mod_params::{ModulationParams, PacketParams, RadioError};
use crate::mod_traits::InterfaceVariant;
use crate::sx128x::radio_kind_params::{RampTime, Register, Sx128xVariant, spreading_factor_value};
use crate::sx128x::{SX1282_RSSI_OFFSET, Sx128x};
use embedded_hal_async::spi::SpiDevice;
use lora_modulation::Bandwidth;

use super::radio_kind_params::OpCode;

/// Sx1280 implements the Sx128xVariant trait
pub struct Sx1280;

impl Sx128xVariant for Sx1280 {
    type Data = ();

    async fn set_packet_params<SPI: SpiDevice<u8>, IV: InterfaceVariant>(
        radio: &mut Sx128x<SPI, IV, Self>,
        pkt_params: &PacketParams,
    ) -> Result<(), RadioError>
    where
        Self: Sized,
    {
        // let modemcfg1 = radio.read_register(Register::RegModemConfig1).await?;

        // let hdr = pkt_params.implicit_header as u8;
        // let crc = pkt_params.crc_on as u8;

        // let cfg1 = (modemcfg1 & 0b1111_1001) | (hdr << 2) | (crc << 1);
        // radio.write_register(Register::RegModemConfig1, cfg1).await?;
        todo!();
        Ok(())
    }

    async fn rssi_offset<SPI: SpiDevice<u8>, IV: InterfaceVariant>(
        _: &mut Sx128x<SPI, IV, Self>,
    ) -> Result<i16, RadioError> {
        Ok(SX1282_RSSI_OFFSET)
    }

    async fn set_tx_continuous_wave_mode<SPI: SpiDevice<u8>, IV: InterfaceVariant>(
        _: &mut Sx128x<SPI, IV, Self>,
    ) -> Result<(), RadioError> {
        todo!()
    }
}
