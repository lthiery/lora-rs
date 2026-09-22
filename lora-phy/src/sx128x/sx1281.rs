use crate::mod_params::{ModulationParams, PacketParams, RadioError};
use crate::mod_traits::InterfaceVariant;
use crate::sx128x::radio_kind_params::{
    OcpTrim, PaConfig, PaDac, RampTime, Register, Sx128xVariant,  spreading_factor_value,
};
use crate::sx128x::{
    SX1286_RF_MID_BAND_THRESH, SX1286_RSSI_OFFSET_HF, SX1286_RSSI_OFFSET_LF, Sx128x, pll_step_to_freq,
};
use embedded_hal_async::spi::SpiDevice;
use lora_modulation::Bandwidth;

/// Sx1281 implements the Sx128xVariant trait
pub struct Sx1281;

#[derive(Default)]
pub struct Sx1281Data {
    /// flag to indicate the Errata 2.1: Sensitivity optimization with 500 kHz bandwidth is required
    sensitivity_quirk: bool,
}

impl Sx128xVariant for Sx1281 {
    type Data = Sx1281Data;



    async fn set_packet_params<SPI: SpiDevice<u8>, IV: InterfaceVariant>(
        radio: &mut Sx128x<SPI, IV, Self>,
        pkt_params: &PacketParams,
    ) -> Result<(), RadioError> {
        // let mut config_1 = radio.read_register(Register::RegModemConfig1).await?;

        // if pkt_params.implicit_header {
        //     config_1 |= 0x01u8;
        // } else {
        //     config_1 &= 0xfeu8;
        // }
        // radio.write_register(Register::RegModemConfig1, config_1).await?;

        // let mut config_2 = radio.read_register(Register::RegModemConfig2).await?;
        // if pkt_params.crc_on {
        //     config_2 |= 0x04u8;
        // } else {
        //     config_2 &= 0xfbu8;
        // }
        // radio.write_register(Register::RegModemConfig2, config_2).await?;
        todo!();
        Ok(())
    }

    async fn rssi_offset<SPI: SpiDevice<u8>, IV: InterfaceVariant>(
        radio: &mut Sx128x<SPI, IV, Self>,
    ) -> Result<i16, RadioError> {
        // let frequency_in_hz = {
        //     // TODO: Keep frequency in radio settings?
        //     let msb = radio.read_register(Register::RegFrfMsb).await? as u32;
        //     let mid = radio.read_register(Register::RegFrfMid).await? as u32;
        //     let lsb = radio.read_register(Register::RegFrfLsb).await? as u32;

        //     pll_step_to_freq((msb << 16) + (mid << 8) + lsb)
        // };

        // if frequency_in_hz > SX1286_RF_MID_BAND_THRESH {
        //     Ok(SX1286_RSSI_OFFSET_HF)
        // } else {
        //     Ok(SX1286_RSSI_OFFSET_LF)
        // }
        todo!();
    }

    async fn set_tx_continuous_wave_mode<SPI: SpiDevice<u8>, IV: InterfaceVariant>(
        radio: &mut Sx128x<SPI, IV, Self>,
    ) -> Result<(), RadioError> {
        // radio.intf.iv.enable_rf_switch_tx().await?;
        // let pa_config = radio.read_register(Register::RegPaConfig).await?;
        // let new_pa_config = pa_config | 0b1000_0000;
        // radio.write_register(Register::RegPaConfig, new_pa_config).await?;
        // radio.write_register(Register::RegOpMode, 0b1000_0011).await?;
        // let modem_config = radio.read_register(Register::RegModemConfig2).await?;
        // let new_modem_config = modem_config | 0b0000_1000;
        // radio
        //     .write_register(Register::RegModemConfig2, new_modem_config)
        //     .await?;
        todo!();
        Ok(())
    }
}
