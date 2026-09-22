mod radio_kind_params;
mod sx1280;
pub use sx1280::Sx1280;
mod sx1281;
pub use sx1281::Sx1281;
#[cfg(test)]
mod test;

use embedded_hal_async::delay::DelayNs;
use embedded_hal_async::spi::*;
use radio_kind_params::*;

use crate::lr1110::radio_kind_params::coding_rate_value;
use crate::mod_params::*;
use crate::mod_traits::IrqState;
use crate::{InterfaceVariant, RadioKind, SpiInterface};

// TCXO flag
const TCXO_FOR_OSCILLATOR: u8 = 0x10u8;

// Limits for preamble detection window in single reception mode
const SX128X_MIN_LORA_SYMB_NUM_TIMEOUT: u16 = 4;
const SX128X_MAX_LORA_SYMB_NUM_TIMEOUT: u16 = 1023;

// Constant values need to compute the RSSI value
const SX1282_RSSI_OFFSET: i16 = -139;
const SX1286_RSSI_OFFSET_LF: i16 = -164;
const SX1286_RSSI_OFFSET_HF: i16 = -157;
const SX1286_RF_MID_BAND_THRESH: u32 = 525_000_000;

const RADIOLIB_SX128X_CRYSTAL_FREQ: f32 = 52_000_000.0;
const RADIOLIB_SX128X_DIV_EXPONENT: usize = 18;

// Frequency synthesizer step: FXOSC (32 MHz) / 524288 (2^19) = 61.03515625 Hz
fn freq_to_pll_step(freq_in_hz: u32) -> u32 {
    // Full-precision integer form of freq / 61.03515625. The previous
    // truncate-then-shift shortcut zeroed the low 8 pll-step bits, putting
    // fractional-MHz channels (868.1, 903.9, ...) up to ~15 kHz off.
    (((freq_in_hz as u64) << 19) / 32_000_000) as u32
}

fn pll_step_to_freq(pll_step: u32) -> u32 {
    (((pll_step as u64) * 32_000_000) >> 19) as u32
}

// RSSI requires linearization when SNR >= 0
// Section 3.5.5 - Note 3
fn linearize_rssi(rssi: u8) -> i16 {
    // Integer approximation for RSSI * 16.0 / 15.0
    // General formula for integer division with rounding:
    // x / d == floor((x + floor(d / 2)) / d), when d > 0
    const DIVISOR: i16 = 15;
    (rssi as i16 * 16 + (DIVISOR / 2)) / DIVISOR
}

/// Configuration for SX128x-based boards
pub struct Config<C: Sx128xVariant> {
    /// LoRa chip used on specific board
    pub chip: C,
    /// Whether board is using crystal oscillator or external clock
    pub tcxo_used: bool,
    /// TODO
    pub tx_boost: bool,
    /// TODO
    pub rx_boost: bool,
}

/// Base for the RadioKind implementation for the LoRa chip kind and board type
pub struct Sx128x<SPI, IV, C: Sx128xVariant + Sized> {
    intf: SpiInterface<SPI, IV>,
    config: Config<C>,
    data: C::Data,
}

impl<SPI, IV, C> Sx128x<SPI, IV, C>
where
    SPI: SpiDevice<u8>,
    IV: InterfaceVariant,
    C: Sx128xVariant,
{
    /// Create an instance of the RadioKind implementation for the LoRa chip kind and board type
    pub fn new(spi: SPI, iv: IV, config: Config<C>) -> Self {
        let intf = SpiInterface::new(spi, iv);
        Self {
            intf,
            config,
            data: Default::default(),
        }
    }

    // Utility functions
    async fn write_register(&mut self, register: Register, value: u8) -> Result<(), RadioError> {
        let write_buffer = [OpCode::WriteRegister as u8, register.addr1(), register.addr2(), value];
        self.intf.write(&write_buffer, false).await
    }

    async fn read_register(&mut self, register: Register) -> Result<u8, RadioError> {
        let write_buffer = [OpCode::WriteRegister as u8, register.addr1(), register.addr2()];
        let mut read_buffer = [0x00u8];
        self.intf.read(&write_buffer, &mut read_buffer).await?;
        Ok(read_buffer[0])
    }

    async fn read_buffer(&mut self, register: Register, buf: &mut [u8]) -> Result<(), RadioError> {
        let write_buffer = [OpCode::WriteRegister as u8, register.addr1(), register.addr2()];
        self.intf.read(&write_buffer, buf).await
    }

    async fn write_buffer(&mut self, register: Register, buf: &[u8]) -> Result<(), RadioError> {
        let write_buffer = [OpCode::WriteRegister as u8, register.addr1(), register.addr2()];
        self.intf.write_with_payload(&write_buffer, buf, false).await
    }

    // Set the number of symbols the radio will wait to detect a reception (up to 1023 symbols)
    async fn set_lora_symbol_num_timeout(&mut self, symbol_num: u16) -> Result<(), RadioError> {
        let val = symbol_num.min(SX128X_MAX_LORA_SYMB_NUM_TIMEOUT);

        // let symbol_num_msb = ((val >> 8) & 0x03) as u8;
        // let symbol_num_lsb = (val & 0xff) as u8;
        // let mut config_2 = self.read_register(Register::RegModemConfig2).await?;
        // config_2 = (config_2 & 0xfcu8) | symbol_num_msb;
        // self.write_register(Register::RegModemConfig2, config_2).await?;
        // self.write_register(Register::RegSymbTimeoutLsb, symbol_num_lsb).await
        todo!();
    }

    // Set the over current protection (mA) on the radio
    async fn set_ocp(&mut self, ocp_trim: OcpTrim) -> Result<(), RadioError> {
        todo!();
        //self.write_register(Register::RegOcp, ocp_trim.value()).await
    }

    #[cfg(test)]
    fn take_spi(self) -> SPI {
        self.intf.spi
    }

    #[cfg(test)]
    fn spi_mut(&mut self) -> &mut SPI {
        &mut self.intf.spi
    }

    async fn set_packet_type(&mut self, packet_type: PacketType) -> Result<(), RadioError> {
        let write_buffer = [OpCode::SetPacketType as u8, packet_type as u8];
        self.intf.write(&write_buffer, true).await
    }

    fn preamble_length_value(&self, preamble_length: u16) -> Result<u8, RadioError> {
        // check preamble length is even - no point even trying odd numbers
        if preamble_length % 2 != 0 {
            return Err(RadioError::InvalidConfiguration);
        }
        for e in 1..16 {
            for m in 1..16 {
                let len = m * (1 << e);
                if len >= preamble_length {
                    return Ok((e << 4 | m) as u8);
                }
            }
        }
        Err(RadioError::InvalidConfiguration)
    }
}

impl<SPI, IV, C> RadioKind for Sx128x<SPI, IV, C>
where
    SPI: SpiDevice<u8>,
    IV: InterfaceVariant,
    C: Sx128xVariant,
{
    // The sx128x drives its single-receive timeout off SymbTimeout, which
    // needs headroom over the 8-symbol LoRaWAN preamble to latch reliably; 6
    // (the trait default) is too short and drops downlinks at higher rates.
    const DEFAULT_MIN_RX_SYMBOLS: u16 = 8;

    // RegSymbTimeout is 10 bits and there is no wall-clock RX timer to fall
    // back on, so longer windows clamp (SUPPORTS_TIMED_SINGLE_RX stays false).
    const MAX_SINGLE_RX_SYMBOLS: u16 = SX128X_MAX_LORA_SYMB_NUM_TIMEOUT;

    async fn init_lora(&mut self, sync_word: u16) -> Result<(), RadioError> {
        let firmware_version = self.read_register(Register::FirmwareVersions).await?;
        debug!("Detected sx128x firmware version v{}", firmware_version);
        self.set_packet_type(PacketType::LoRa).await?;
        self.set_lora_sync_word(sync_word).await?;
        self.set_tx_rx_buffer_base_address(0, 0).await?;
        Ok(())
    }

    async fn set_lora_sync_word(&mut self, sync_word: u16) -> Result<(), RadioError> {
        let sync_word_buffer = [(sync_word >> 8) as u8, sync_word as u8];
        self.write_buffer(Register::LoRaSyncWord0, &sync_word_buffer).await
    }

    fn create_modulation_params(
        &self,
        spreading_factor: SpreadingFactor,
        bandwidth: Bandwidth,
        coding_rate: CodingRate,
        frequency_in_hz: u32,
    ) -> Result<ModulationParams, RadioError> {
        // Parameter validation
        spreading_factor_value(spreading_factor)?;
        coding_rate_value(coding_rate)?;
        bandwidth_value(bandwidth)?;
        let low_data_rate_optimize = 0x00u8;
        Ok(ModulationParams {
            spreading_factor,
            bandwidth,
            coding_rate,
            low_data_rate_optimize,
            frequency_in_hz,
        })
    }

    async fn set_modulation_params(&mut self, mdltn_params: &ModulationParams) -> Result<(), RadioError> {
        let mod_param_1 = spreading_factor_value(mdltn_params.spreading_factor)?;
        let mod_param_2 = bandwidth_value(mdltn_params.bandwidth)?;
        let mod_param_3 = coding_rate_value(mdltn_params.coding_rate)?;
        let buffer = [OpCode::SetModulationParams as u8, mod_param_1, mod_param_2, mod_param_3];
        self.intf.write(&buffer, true).await
    }

    fn create_packet_params(
        &self,
        preamble_length: u16,
        implicit_header: bool,
        payload_length: u8,
        crc_on: bool,
        iq_inverted: bool,
        modulation_params: &ModulationParams,
    ) -> Result<PacketParams, RadioError> {
        // Parameter validation
        if (modulation_params.spreading_factor == SpreadingFactor::_6) && !implicit_header {
            return Err(RadioError::InvalidSF6ExplicitHeaderRequest);
        }

        Ok(PacketParams {
            preamble_length,
            implicit_header,
            payload_length,
            crc_on,
            iq_inverted,
        })
    }

    async fn reset(&mut self, delay: &mut impl DelayNs) -> Result<(), RadioError> {
        self.intf.iv.reset(delay).await?;
        self.set_sleep(false, delay).await?; // ensure sleep mode is entered so that the LoRa mode bit is set
        Ok(())
    }

    async fn ensure_ready(&mut self, _mode: RadioMode) -> Result<(), RadioError> {
        // TODO
        Ok(())
    }

    async fn set_standby(&mut self) -> Result<(), RadioError> {
        self.intf.iv.disable_rf_switch().await?;
        let buf = [
            OpCode::SetStandby as u8,
            0x01,
            if self.config.tcxo_used {
                StandbyConfig::Xosc
            } else {
                StandbyConfig::Rc
            } as u8,
        ];
        self.intf.write(&buf, true).await
    }

    async fn set_sleep(&mut self, _warm_start_if_possible: bool, _delay: &mut impl DelayNs) -> Result<(), RadioError> {
        // Warm start is unavailable for sx128x
        self.intf.iv.disable_rf_switch().await?;
        // Table 11-17: allow for faster SLEEP to STDBY_RC transition
        let buf = [OpCode::SetSleep as u8, 0x01];
        self.intf.write(&buf, true).await?;
        Ok(())
    }

    async fn set_tx_rx_buffer_base_address(
        &mut self,
        tx_base_addr: usize,
        rx_base_addr: usize,
    ) -> Result<(), RadioError> {
        if tx_base_addr > 255 || rx_base_addr > 255 {
            return Err(RadioError::InvalidBaseAddress(tx_base_addr, rx_base_addr));
        }
        let buf = [
            OpCode::SetBufferBaseAddress as u8,
            tx_base_addr as u8,
            rx_base_addr as u8,
        ];
        self.intf.write(&buf, true).await
    }

    // Set parameters associated with power for a send operation.
    //   p_out                   desired RF output power (dBm)
    //   mdltn_params            needed for a power vs channel frequency validation
    //   is_tx_prep              indicates which ramp up time to use
    async fn set_tx_power_and_ramp_time(
        &mut self,
        p_out: i32,
        _mdltn_params: Option<&ModulationParams>,
        is_tx_prep: bool,
    ) -> Result<(), RadioError> {
        debug!("tx power = {}", p_out);
        // 4us, as in ELRS
        let ramp_time = RampTime::Ramp04Us;
        let power = p_out + 18;
        let write_buffer = [OpCode::SetTxParams as u8, power as u8, ramp_time as u8];
        self.intf.write(&write_buffer, true).await
    }

    async fn set_packet_params(&mut self, pkt_params: &PacketParams) -> Result<(), RadioError> {
        let buffer = [
            OpCode::SetPacketParams as u8,
            self.preamble_length_value(pkt_params.preamble_length)?,
            if pkt_params.implicit_header { 0x80 } else { 0x00 },
            pkt_params.payload_length,
            if pkt_params.crc_on { 0x20 } else { 0x00 },
            if pkt_params.iq_inverted { 0x00 } else { 0x40 },
            0,
            0,
        ];
        self.intf.write(&buffer, true).await
    }

    // Calibrate the image rejection based on the given frequency
    async fn calibrate_image(&mut self, _frequency_in_hz: u32) -> Result<(), RadioError> {
        // An automatic process, but can set bit ImageCalStart in RegImageCal, when the device is in Standby mode.
        Ok(())
    }

    async fn set_channel(&mut self, frequency_in_hz: u32) -> Result<(), RadioError> {
        let frequency_in_hz = frequency_in_hz as f32;
        let exp = (1 << RADIOLIB_SX128X_DIV_EXPONENT) as f32;
        // Section 4.3
        let pll = (frequency_in_hz * exp / RADIOLIB_SX128X_CRYSTAL_FREQ) as u32;
        debug!("channel = {}, pll = {}", frequency_in_hz, pll);
        let pll_bytes = pll.to_be_bytes();
        let buffer = [OpCode::SetRfFrequency as u8, pll_bytes[1], pll_bytes[2], pll_bytes[3]];
        self.intf.write(&buffer, true).await
    }

    async fn set_payload(&mut self, payload: &[u8]) -> Result<(), RadioError> {
        let write_buffer = [OpCode::WriteBuffer as u8, 0];
        self.intf.write_with_payload(&write_buffer, payload, true).await
    }

    async fn do_tx(&mut self) -> Result<(), RadioError> {
        self.intf.iv.enable_rf_switch_tx().await?;
        // Table 11-22, no timeout
        let write_buffer = [OpCode::SetTx as u8, 0, 0, 0];
        self.intf.write(&write_buffer, true).await
    }

    async fn do_rx(&mut self, rx_mode: RxMode) -> Result<(), RadioError> {
        // let (num_symbols, mode) = match rx_mode {
        //     RxMode::DutyCycle(_) => Err(RadioError::DutyCycleUnsupported),
        //     RxMode::Single(ns) => Ok((ns.max(SX128X_MIN_LORA_SYMB_NUM_TIMEOUT), LoRaMode::RxSingle)),
        //     RxMode::SingleMs(_) => Err(RadioError::TimedSingleRxUnsupported),
        //     RxMode::Continuous => Ok((0, LoRaMode::RxContinuous)),
        // }?;

        // self.intf.iv.enable_rf_switch_rx().await?;

        // self.set_lora_symbol_num_timeout(num_symbols).await?;

        // let lna_gain = if self.config.rx_boost {
        //     LnaGain::G1.boosted_value()
        // } else {
        //     LnaGain::G1.value()
        // };
        // self.write_register(Register::RegLna, lna_gain).await?;

        // self.write_register(Register::RegFifoAddrPtr, 0x00u8).await?;

        // // Interrupt flags stay latched until the host clears them by writing a 1
        // // (SX1286 DS §4.1.2.4); entering Rx does not reset them. Clear here so a
        // // flag left over from an earlier operation can't read as a result of this
        // // one; this also covers listen(), which never calls set_irq_params.
        // self.clear_irq_status().await?;

        // self.write_register(Register::RegOpMode, mode.value()).await
        todo!();
    }

    async fn get_rx_payload(
        &mut self,
        rx_pkt_params: &PacketParams,
        receiving_buffer: &mut [u8],
    ) -> Result<u8, RadioError> {
        // let payload_length = if rx_pkt_params.implicit_header {
        //     rx_pkt_params.payload_length
        // } else {
        //     self.read_register(Register::RegRxNbBytes).await?
        // };
        // if (payload_length as usize) > receiving_buffer.len() {
        //     return Err(RadioError::PayloadSizeMismatch(
        //         payload_length as usize,
        //         receiving_buffer.len(),
        //     ));
        // }
        // let fifo_addr = self.read_register(Register::RegFifoRxCurrentAddr).await?;
        // self.write_register(Register::RegFifoAddrPtr, fifo_addr).await?;
        // self.read_buffer(Register::RegFifo, &mut receiving_buffer[0..payload_length as usize])
        //     .await?;
        // self.write_register(Register::RegFifoAddrPtr, 0x00u8).await?;

        // Ok(payload_length)
        todo!();
    }

    async fn get_rx_packet_status(&mut self) -> Result<PacketStatus, RadioError> {
        // let snr = {
        //     let packet_snr = self.read_register(Register::RegPktSnrValue).await?;
        //     packet_snr as i8 as i16 / 4
        // };

        // let rssi = {
        //     let packet_rssi = self.read_register(Register::RegPktRssiValue).await?;

        //     let rssi_offset = C::rssi_offset(self).await?;

        //     // Section 5.5.5: the 16/15 linearization applies to the raw
        //     // packet RSSI in both branches (the reference driver and
        //     // LoRaMac-node agree; only the negative-SNR term differs)
        //     if snr >= 0 {
        //         rssi_offset + linearize_rssi(packet_rssi)
        //     } else {
        //         rssi_offset + linearize_rssi(packet_rssi) + snr
        //     }
        // };

        // Ok(PacketStatus { rssi, snr })
        todo!();
    }

    async fn get_rssi(&mut self) -> Result<i16, RadioError> {
        // let rssi_value = self.read_register(Register::RegRssiValue).await?;
        // let rssi_offset = C::rssi_offset(self).await?;
        // Ok(rssi_offset + rssi_value as i16)
        todo!();
    }

    async fn do_cad(&mut self, _mdltn_params: &ModulationParams) -> Result<(), RadioError> {
        // self.intf.iv.enable_rf_switch_rx().await?;

        // let mut lna_gain_final = LnaGain::G1.value();
        // if self.config.rx_boost {
        //     lna_gain_final = LnaGain::G1.boosted_value();
        // }
        // self.write_register(Register::RegLna, lna_gain_final).await?;

        // self.write_register(Register::RegOpMode, LoRaMode::Cad.value()).await
        todo!();
    }

    // Set the IRQ mask to disable unwanted interrupts,
    // enable interrupts on DIO pins (sx128x has multiple),
    // and allow interrupts.
    async fn set_irq_params(&mut self, _radio_mode: Option<RadioMode>) -> Result<(), RadioError> {
        self.clear_irq_status().await?;
        // Taken from ELRS
        let irq_mask = IrqMask::TxDone.value()
            | IrqMask::RxDone.value()
            | IrqMask::SyncWordValid.value()
            | IrqMask::SyncWordError.value()
            | IrqMask::CrcError.value();
        let dio1_mask = IrqMask::TxDone.value() | IrqMask::RxDone.value();
        let dio2_mask: u16 = 0;
        let dio3_mask: u16 = 0;
        let mut buffer = [OpCode::SetDioIrqParams as u8, 0, 0, 0, 0, 0, 0, 0, 0];
        buffer[1..3].copy_from_slice(&irq_mask.to_be_bytes());
        buffer[3..5].copy_from_slice(&dio1_mask.to_be_bytes());
        buffer[5..7].copy_from_slice(&dio2_mask.to_be_bytes());
        buffer[7..9].copy_from_slice(&dio3_mask.to_be_bytes());
        self.intf.write(&buffer, true).await
    }

    async fn await_irq(&mut self) -> Result<(), RadioError> {
        self.intf.iv.await_irq().await
    }

    async fn get_irq_state(
        &mut self,
        radio_mode: RadioMode,
        cad_activity_detected: Option<&mut bool>,
    ) -> Result<Option<IrqState>, RadioError> {
        // let irq_flags = self.read_register(Register::RegIrqFlags).await?;
        // match radio_mode {
        //     RadioMode::Transmit => {
        //         if (irq_flags & IrqMask::TxDone.value()) == IrqMask::TxDone.value() {
        //             debug!("TxDone in radio mode {}", radio_mode);
        //             return Ok(Some(IrqState::Done));
        //         }
        //     }
        //     RadioMode::Receive(RxMode::Continuous | RxMode::Single(_) | RxMode::SingleMs(_)) => {
        //         if (irq_flags & IrqMask::RxDone.value()) == IrqMask::RxDone.value() {
        //             debug!("RxDone in radio mode {}", radio_mode);
        //             return Ok(Some(IrqState::Done));
        //         }
        //         if (irq_flags & IrqMask::RxTimeout.value()) == IrqMask::RxTimeout.value() {
        //             debug!("RxTimeout in radio mode {}", radio_mode);
        //             return Err(RadioError::ReceiveTimeout);
        //         }
        //         if IrqMask::HeaderValid.is_set_in(irq_flags) {
        //             debug!("HeaderValid in radio mode {}", radio_mode);
        //             return Ok(Some(IrqState::PreambleReceived));
        //         }
        //     }
        //     RadioMode::ChannelActivityDetection => {
        //         if (irq_flags & IrqMask::CADDone.value()) == IrqMask::CADDone.value() {
        //             debug!("CADDone in radio mode {}", radio_mode);
        //             // TODO: don't like how we mutate the cad_activity_detected parameter
        //             if let Some(cad_activity_detected) = cad_activity_detected {
        //                 // Check if the CAD (Channel Activity Detection) Activity Detected flag is set in irq_flags and then update the reference
        //                 *(cad_activity_detected) =
        //                     (irq_flags & IrqMask::CADActivityDetected.value()) == IrqMask::CADActivityDetected.value();
        //             }

        //             return Ok(Some(IrqState::Done));
        //         }
        //     }
        //     RadioMode::Sleep | RadioMode::Standby | RadioMode::Listen => {
        //         warn!("IRQ during sleep/standby/listen?");
        //     }
        //     RadioMode::FrequencySynthesis => todo!(),
        //     RadioMode::Receive(RxMode::DutyCycle(_)) => todo!(),
        // }

        // // If no specific IRQ condition is met, return None
        // Ok(None)
        todo!();
    }

    async fn clear_irq_status(&mut self) -> Result<(), RadioError> {
        let buffer = [OpCode::ClrIrqStatus as u8, 0xff, 0xff];
        self.intf.write(&buffer, true).await
    }

    /// Process the radio IRQ. Log unexpected interrupts. Packets from other
    /// devices can cause unexpected interrupts.
    ///
    /// NB! Do not await this future in a select branch as interrupting it
    /// mid-flow could cause radio lock up.
    async fn process_irq_event(
        &mut self,
        radio_mode: RadioMode,
        cad_activity_detected: Option<&mut bool>,
        clear_interrupts: bool,
    ) -> Result<Option<IrqState>, RadioError> {
        let irq_state = self.get_irq_state(radio_mode, cad_activity_detected).await;
        if clear_interrupts {
            self.clear_irq_status().await?;
        }
        irq_state
    }

    /// Set the LoRa chip into the TxContinuousWave mode
    async fn set_tx_continuous_wave_mode(&mut self) -> Result<(), RadioError> {
        C::set_tx_continuous_wave_mode(self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // FXOSC[32 MHz] * 1000000 (Hz/MHz) / 524288 (2^19)
    const FREQUENCY_SYNTHESIZER_STEP: f64 = 61.03515625;

    #[test]
    fn pll_step_freq_u32_vs_f64() {
        // Simplified integer calculation converges with floating
        // point formula for full and half megahertz values
        const D: u32 = 2;
        for freq in D * 137..=(D * 1020) {
            let f = freq * (1_000_000 / D);

            let pll = freq_to_pll_step(f);
            assert_eq!(pll, (f as f64 / FREQUENCY_SYNTHESIZER_STEP) as u32);
            assert_eq!(pll_step_to_freq(pll), f);
        }
    }

    #[test]
    fn test_rssi_linearization() {
        const DELTA: f32 = 0.5;
        for offset in [SX1282_RSSI_OFFSET, SX1286_RSSI_OFFSET_LF, SX1286_RSSI_OFFSET_HF] {
            for rssi in 0..=u8::MAX {
                let float_rssi = offset as f32 + rssi as f32 * 16.0 / 15.0;
                let approx_rssi = offset + linearize_rssi(rssi);
                let error = float_rssi - approx_rssi as f32;
                assert!(error.abs() < DELTA);
            }
        }
    }
}
