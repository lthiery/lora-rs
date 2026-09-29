mod radio_kind_params;
mod sx1280;
use radio_kind_params::IrqMask::{CrcError, HeaderError};
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
        // After the register, a NOP byte is needed.
        debug!("sx128x::read_register");
        let write_buffer = [OpCode::ReadRegister as u8, register.addr1(), register.addr2(), 0x00];
        let mut read_buffer = [0x00u8; 1];
        self.intf.read(&write_buffer, &mut read_buffer).await?;
        Ok(read_buffer[0])
    }

    async fn read_register_into_buffer(
        &mut self,
        register: Register,
        read_buffer: &mut [u8],
    ) -> Result<(), RadioError> {
        // After the register, a NOP byte is needed.
        debug!("sx128x::read_register");
        let write_buffer = [OpCode::ReadRegister as u8, register.addr1(), register.addr2(), 0x00];
        self.intf.read(&write_buffer, read_buffer).await?;
        Ok(())
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
        let mut write_buffer = [0; 16];
        self.read_register_into_buffer(Register::VersionString, &mut write_buffer)
            .await?;
        debug!("firmware versions: {:?}", write_buffer);
        if b"SX1280" != &write_buffer[0..6] {
            // TODO: maybe introduce explicit error code
            return Err(RadioError::InvalidConfiguration);
        }
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
        info!("set_modulation_params: {:02X}", OpCode::SetModulationParams as u8);
        let mod_param_1 = spreading_factor_value(mdltn_params.spreading_factor)?;
        let mod_param_2 = bandwidth_value(mdltn_params.bandwidth)?;
        let mod_param_3 = coding_rate_value(mdltn_params.coding_rate)?;
        let buffer = [OpCode::SetModulationParams as u8, mod_param_1, mod_param_2, mod_param_3];
        self.intf.write(&buffer, true).await?;
        // section 14.4.1, before table 14-48
        self.write_register(
            Register::SfAdditionalConfiguration,
            match mdltn_params.spreading_factor {
                SpreadingFactor::_5 | SpreadingFactor::_6 => 0x1E,
                SpreadingFactor::_7 | SpreadingFactor::_8 => 0x37,
                SpreadingFactor::_9 | SpreadingFactor::_10 | SpreadingFactor::_11 | SpreadingFactor::_12 => 0x32,
            },
        )
        .await?;
        self.write_register(Register::FrequencyErrorCorrection, 0x01).await?;
        // These two follow the RadioLib
        let buffer = [OpCode::SetCadParams as u8, CadSymbolNum::_8 as u8];
        self.intf.write(&buffer, true).await?;
        // 14.7.2: Assumes we don't have an extra inductor, might need confuration.
        let buffer = [OpCode::SetRegulatorMode as u8, 0x00];
        self.intf.write(&buffer, true).await?;
        // TODO: limit payload lenth, after table 14-49.
        Ok(())
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
        debug!("sx128x::ensure_ready");
        self.set_standby().await?;
        Ok(())
    }

    async fn set_standby(&mut self) -> Result<(), RadioError> {
        debug!("sx128x::set_standby");
        self.intf.iv.disable_rf_switch().await?;
        // send a NOP to wake up
        self.intf.write(&[0], true).await?;
        let buf = [
            OpCode::SetStandby as u8,
            if self.config.tcxo_used {
                StandbyConfig::Xosc
            } else {
                StandbyConfig::Rc
            } as u8,
        ];
        let mut read_buf = [0; 0];
        // TODO: evaluate status as in RadioLib
        let status = self.intf.read_with_status(&buf, &mut read_buf).await?;
        info!("status: {:02X}", status);
        Ok(())
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
        debug!("sx128x::set_tx_rx_buffer_base_address");
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
        debug!("sx128x::set_tx_power_and_ramp_time:tx power = {}", p_out);
        // 10us, as in Radiolib
        let ramp_time = RampTime::Ramp10Us;
        let power = p_out + 18;
        let write_buffer = [OpCode::SetTxParams as u8, power as u8, ramp_time as u8];
        self.intf.write(&write_buffer, true).await
    }

    async fn set_packet_params(&mut self, pkt_params: &PacketParams) -> Result<(), RadioError> {
        debug!("sx128x::set_packet_params");
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
        self.set_tx_rx_buffer_base_address(0, 0).await?;
        self.clear_irq_status().await?;
        let (period_base, period_base_count) = match rx_mode {
            RxMode::Single(timeout) => (PeriodBase::_15_625us, timeout),
            RxMode::SingleMs(ms) => (PeriodBase::_1ms, ms as u16),
            RxMode::Continuous => (PeriodBase::_1ms, 0xFFFF),
            RxMode::DutyCycle(_duty_cycle_params) => return Err(RadioError::InvalidConfiguration),
        };
        let pbb = period_base_count.to_be_bytes();
        let buffer = [OpCode::SetRx as u8, period_base as u8, pbb[0], pbb[1]];
        self.intf.write(&buffer, true).await
    }

    async fn get_rx_payload(
        &mut self,
        rx_pkt_params: &PacketParams,
        receiving_buffer: &mut [u8],
    ) -> Result<u8, RadioError> {
        let payload_length = if rx_pkt_params.implicit_header {
            rx_pkt_params.payload_length
        } else {
            let write_buffer = [OpCode::GetRxBufferStatus as u8, 0];
            let mut read_buffer = [0; 2];
            self.intf.read(&write_buffer, &mut read_buffer).await?;
            read_buffer[0]
        };
        if (payload_length as usize) > receiving_buffer.len() {
            return Err(RadioError::PayloadSizeMismatch(
                payload_length as usize,
                receiving_buffer.len(),
            ));
        }
        // TODO: So far hard-coded
        let fifo_offset = 0;
        let write_buffer = [OpCode::ReadBuffer as u8, fifo_offset, 0];
        let mut read_buffer = [0; 256];
        self.intf
            .read(&write_buffer, &mut read_buffer[0..payload_length as usize])
            .await?;
        receiving_buffer[0..payload_length as usize].copy_from_slice(&mut read_buffer[0..payload_length as usize]);
        Ok(payload_length)
    }

    async fn get_rx_packet_status(&mut self) -> Result<PacketStatus, RadioError> {
        let write_buffer = [OpCode::GetPacketStatus as u8, 0];
        let mut read_buffer = [0; 5];
        self.intf.read(&write_buffer, &mut read_buffer).await?;
        let rssi = read_buffer[0] as i16;
        let snr = read_buffer[1] as i16;
        Ok(PacketStatus { rssi, snr })
    }

    async fn get_rssi(&mut self) -> Result<i16, RadioError> {
        Ok(self.get_rx_packet_status().await?.rssi)
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
    async fn set_irq_params(&mut self, radio_mode: Option<RadioMode>) -> Result<(), RadioError> {
        debug!("sx128x::set_irq_params");
        self.clear_irq_status().await?;
        if let Some(radio_mode) = radio_mode {
            match radio_mode {
                RadioMode::Sleep | RadioMode::Standby => {
                    let buffer = irq_dio1_buffer_from_params(0, 0, 0, 0);
                    self.intf.write(&buffer, true).await?
                }
                RadioMode::FrequencySynthesis => todo!(),
                RadioMode::Transmit => {
                    // Taken from RadioLib
                    let irq_mask = IrqMask::TxDone.value()
                        | IrqMask::RxDone.value()
                        | IrqMask::SyncWordValid.value()
                        | IrqMask::SyncWordError.value()
                        | IrqMask::CrcError.value();
                    let dio1_mask = IrqMask::TxDone.value() | IrqMask::RxDone.value();
                    let dio2_mask: u16 = 0;
                    let dio3_mask: u16 = 0;
                    let buffer = irq_dio1_buffer_from_params(irq_mask, dio1_mask, dio2_mask, dio3_mask);
                    self.intf.write(&buffer, true).await?
                }
                RadioMode::Receive(rx_mode) => match rx_mode {
                    RxMode::Single(_) => todo!(),
                    RxMode::SingleMs(_) => todo!(),
                    RxMode::Continuous => {
                        // Taken from RadioLib
                        let irq_mask = IrqMask::RxDone.value()
                            | IrqMask::HeaderValid.value()
                            | IrqMask::HeaderError.value()
                            | IrqMask::CrcError.value()
                            | IrqMask::RxTxTimeout.value();
                        let dio1_mask = IrqMask::RxDone.value() | IrqMask::RxTxTimeout.value();
                        let dio2_mask: u16 = 0;
                        let dio3_mask: u16 = 0;
                        let buffer = irq_dio1_buffer_from_params(irq_mask, dio1_mask, dio2_mask, dio3_mask);
                        self.intf.write(&buffer, true).await?
                    }
                    RxMode::DutyCycle(_duty_cycle_params) => todo!(),
                },
                RadioMode::Listen => todo!(),
                RadioMode::ChannelActivityDetection => todo!(),
            }
        }
        Ok(())
    }

    async fn await_irq(&mut self) -> Result<(), RadioError> {
        self.intf.iv.await_irq().await
    }

    async fn get_irq_state(
        &mut self,
        radio_mode: RadioMode,
        cad_activity_detected: Option<&mut bool>,
    ) -> Result<Option<IrqState>, RadioError> {
        // Needs a NOP, see 11.9.2
        let write_buffer = [OpCode::GetIrqStatus as u8, 0];
        let mut read_buffer = [0; 2];
        self.intf.read(&write_buffer, &mut read_buffer).await?;
        let irq_flags = u16::from_be_bytes(read_buffer);
        match radio_mode {
            RadioMode::Receive(RxMode::Continuous | RxMode::Single(_) | RxMode::SingleMs(_)) => {
                if (irq_flags & IrqMask::RxDone.value()) == IrqMask::RxDone.value() {
                    debug!("RxDone in radio mode {}", radio_mode);
                    return Ok(Some(IrqState::Done));
                }
                if (irq_flags & IrqMask::RxTxTimeout.value()) == IrqMask::RxTxTimeout.value() {
                    debug!("RxTxTimeout in radio mode {}", radio_mode);
                    return Err(RadioError::ReceiveTimeout);
                }
                if IrqMask::HeaderValid.is_set_in(irq_flags) {
                    debug!("HeaderValid in radio mode {}", radio_mode);
                    return Ok(Some(IrqState::PreambleReceived));
                }
            }
            RadioMode::Transmit => {
                if (irq_flags & IrqMask::TxDone.value()) == IrqMask::TxDone.value() {
                    debug!("TxDone in radio mode {}", radio_mode);
                    return Ok(Some(IrqState::Done));
                }
            }
            // RadioMode::ChannelActivityDetection => {
            //     if (irq_flags & IrqMask::CADDone.value()) == IrqMask::CADDone.value() {
            //         debug!("CADDone in radio mode {}", radio_mode);
            //         // TODO: don't like how we mutate the cad_activity_detected parameter
            //         if let Some(cad_activity_detected) = cad_activity_detected {
            //             // Check if the CAD (Channel Activity Detection) Activity Detected flag is set in irq_flags and then update the reference
            //             *(cad_activity_detected) =
            //                 (irq_flags & IrqMask::CADActivityDetected.value()) == IrqMask::CADActivityDetected.value();
            //         }

            //         return Ok(Some(IrqState::Done));
            //     }
            // }
            // RadioMode::Sleep | RadioMode::Standby | RadioMode::Listen => {
            //     warn!("IRQ during sleep/standby/listen?");
            // }
            // RadioMode::FrequencySynthesis => todo!(),
            // RadioMode::Receive(RxMode::DutyCycle(_)) => todo!(),
            e @ _ => warn!("Unknown radio mode: {:?}!", e),
        }

        // If no specific IRQ condition is met, return None
        Ok(None)
    }

    async fn clear_irq_status(&mut self) -> Result<(), RadioError> {
        debug!("sx128x::clear_irq_status");
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

fn irq_dio1_buffer_from_params(irq_mask: u16, dio1_mask: u16, dio2_mask: u16, dio3_mask: u16) -> [u8; 9] {
    let mut buffer = [OpCode::SetDioIrqParams as u8, 0, 0, 0, 0, 0, 0, 0, 0];
    buffer[1..3].copy_from_slice(&irq_mask.to_be_bytes());
    buffer[3..5].copy_from_slice(&dio1_mask.to_be_bytes());
    buffer[5..7].copy_from_slice(&dio2_mask.to_be_bytes());
    buffer[7..9].copy_from_slice(&dio3_mask.to_be_bytes());
    buffer
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
