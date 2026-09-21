use crate::mod_params::{ModulationParams, PacketParams, RadioError};
use crate::mod_traits::InterfaceVariant;
use crate::sx128x::radio_kind_params::{RampTime, Register, Sx128xVariant, coding_rate_value, spreading_factor_value};
use crate::sx128x::{SX1282_RSSI_OFFSET, Sx128x};
use embedded_hal_async::spi::SpiDevice;
use lora_modulation::Bandwidth;

/// Sx1280 implements the Sx128xVariant trait
pub struct Sx1280;

impl Sx128xVariant for Sx1280 {
    type Data = ();

    async fn init_lora<SPI: SpiDevice<u8>, IV: InterfaceVariant>(
        _radio: &mut Sx128x<SPI, IV, Self>,
        _sync_word: u8,
    ) -> Result<(), RadioError> {
        Ok(())
    }

    fn bandwidth_value(bw: Bandwidth) -> Result<u8, RadioError> {
        match bw {
            Bandwidth::_200KHz => Ok(0x34),
            Bandwidth::_400KHz => Ok(0x26),
            Bandwidth::_800KHz => Ok(0x18),
            Bandwidth::_1600KHz => Ok(0x0A),
            _ => Err(RadioError::InvalidBandwidthForFrequency),
        }
    }

    async fn set_tx_power<SPI: SpiDevice<u8>, IV: InterfaceVariant>(
        radio: &mut Sx128x<SPI, IV, Self>,
        p_out: i32,
        tx_boost: bool,
    ) -> Result<(), RadioError> {
        todo!();
        Ok(())
    }

    fn ramp_value(ramp_time: RampTime) -> u8 {
        // Sx1280 - default: 0x19
        // [4]: LowPnTxPllOff - use higher power, lower phase noise PLL
        //      only when the transmitter is used (default: 1)
        //      0 - Standard PLL used in Rx mode, Lower PN PLL in Tx
        //      1 - Standard PLL used in both Tx and Rx modes
        ramp_time.value() | (1 << 4)
    }

    async fn set_modulation_params<SPI: SpiDevice<u8>, IV: InterfaceVariant>(
        radio: &mut Sx128x<SPI, IV, Self>,
        mdltn_params: &ModulationParams,
    ) -> Result<(), RadioError> {
        // let bw_val = Self::bandwidth_value(mdltn_params.bandwidth)?;
        // let sf_val = spreading_factor_value(mdltn_params.spreading_factor)?;

        // let cfg1 = radio.read_register(Register::RegModemConfig1).await?;
        // let ldro = mdltn_params.low_data_rate_optimize;
        // let cr_val = coding_rate_value(mdltn_params.coding_rate)?;
        // let val = (cfg1 & 0b110) | (bw_val << 6) | (cr_val << 3) | ldro;
        // radio.write_register(Register::RegModemConfig1, val).await?;
        // let cfg2 = radio.read_register(Register::RegModemConfig2).await?;
        // let val = (cfg2 & 0b1111) | (sf_val << 4);
        // radio.write_register(Register::RegModemConfig2, val).await?;
        todo!();
        Ok(())
    }

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
