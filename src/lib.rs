use serialport::{DataBits, FlowControl, Parity, SerialPort, StopBits};
use tokio::io::{AsyncReadExt,AsyncWriteExt};
type Port = Box<dyn SerialPort>;
const PACKET_SIZE: usize = 128;
#[derive(Debug)]
enum YmodemControlCode {
    Soh = 0x01,
    Stx,
    Eot = 0x04,
    Ack = 0x06,
    Nak = 0x15,
    Can = 0x18,
    C = 0x43,
}
#[derive(std::cmp::PartialEq,Debug)]
pub enum YmodemError {
    InvalidResponse,
    Timeout,
    RequestReSend,
    SendFailed,
}
impl std::fmt::Display for YmodemError {
  fn fmt(&self, f:  &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
     match self {
         Self::InvalidResponse => write!(f, "Invalid response"),
         Self::Timeout => write!(f, "Timeout"),
         Self::RequestReSend => write!(f, "Request re-send"),
         Self::SendFailed => write!(f, "Send failed"),
     }
  }
}
pub struct YmodemSender<'a> {
    fname: String,
    fdata: &'a [u8],
}

impl<'a> YmodemSender<'a> {
    pub fn new(fname: &str, fdata: &'a [u8]) -> Self {
        Self {
            fname: fname.to_string(),
            fdata,
        }
    }
    fn wait_msg(port: &mut Box<dyn SerialPort>) -> u8 {
      let mut response = [0; 1];
      port.read_exact(&mut response).unwrap();
      response[0]
    }
    fn wait_for_ack(port: &mut Box<dyn SerialPort>) -> Result<(), YmodemError> {
        let response = Self::wait_msg(port);
        if response == YmodemControlCode::Ack as u8 {
            Ok(())
        } else if response == YmodemControlCode::Nak as u8 {
            Err(YmodemError::RequestReSend)
        } else if response == YmodemControlCode::Can as u8 {
            Err(YmodemError::SendFailed)
        } else {
            Err(YmodemError::InvalidResponse)
        }
    }
    fn create_file_header(&self) -> Vec<u8> {
        let mut header = vec![YmodemControlCode::Soh as u8, 0, 255];
        let mut file_info = Vec::new();
        file_info.extend_from_slice(self.fname.as_bytes());
        file_info.push(0); // null terminator

        file_info.extend_from_slice(self.fdata.len().to_string().as_bytes());
        file_info.push(0x20); // null terminator
        let mut block = [0u8; PACKET_SIZE];
        block[..file_info.len()].copy_from_slice(&file_info);
        header.extend_from_slice(&block);
        let crc_value = crc16_ccitt(&block);
        header.push((crc_value >> 8) as u8);
        header.push((crc_value & 0xFF) as u8);
        header
    }
    fn create_data_block(chunk: &[u8], block_number: u8) -> Vec<u8> {
        let mut block = vec![
            YmodemControlCode::Soh as u8, /*STX*/
            block_number,
            !block_number,
        ];
        let mut data = [0u8; PACKET_SIZE];
        data[..chunk.len()].copy_from_slice(chunk);
        block.extend_from_slice(&data);
        // Convert CRC value to little-endian
        let crc_value = crc16_ccitt(&data);
        block.push((crc_value >> 8) as u8);
        block.push((crc_value & 0xFF) as u8);
        block
    }
    fn send_packet(&self, port: &mut Port, packet: &[u8]) -> Result<(), YmodemError> {
        port.write_all(packet).unwrap();
        while let Err(e) = Self::wait_for_ack(&mut *port) {
            if e == YmodemError::RequestReSend {
                port.write_all(packet).unwrap();
            } else {
                return Err(e);
            }
        }
        Ok(())
    }
    pub fn send(&self, port: &mut Port) -> Result<(), YmodemError> {
        let mut response = [0; 1];
        loop {
            port.read_exact(&mut response).unwrap();
            if response[0] == YmodemControlCode::C as u8 {
                break;
            }
        }
        let file_header = self.create_file_header();
        self.send_packet(port, &file_header)?;
        if Self::wait_msg(port) != YmodemControlCode::C as u8 {
          return Err(YmodemError::InvalidResponse)
        }
        for (block_number, chunk) in self.fdata.chunks(PACKET_SIZE).enumerate() {
            let data_block = Self::create_data_block(chunk, (block_number + 1) as u8);
            self.send_packet(port, &data_block)?;
        }
        // EOTの送信
        self.send_packet(port, &[YmodemControlCode::Eot as u8])?;
        if Self::wait_msg(port) != YmodemControlCode::C as u8 {
          return Err(YmodemError::InvalidResponse)
        }
        let data_block = Self::create_data_block(&[0; PACKET_SIZE], 0);
        self.send_packet(port, &data_block)?;
        // 最後のACKを待つ
        println!("YMODEM PASS!");
        Ok(())
    }
// ASYNC CODE
    async fn wait_msg_async(port: &mut serial2_tokio::SerialPort) -> u8 {
      let mut response = [0; 1];
      port.read_exact(&mut response).await.unwrap();
      response[0]
    }
    async fn wait_for_ack_async(port: &mut serial2_tokio::SerialPort) -> Result<(), YmodemError> {
        let response = Self::wait_msg_async(port).await;
        if response == YmodemControlCode::Ack as u8 {
            Ok(())
        } else if response == YmodemControlCode::Nak as u8 {
            Err(YmodemError::RequestReSend)
        } else if response == YmodemControlCode::Can as u8 {
            Err(YmodemError::SendFailed)
        } else {
            Err(YmodemError::InvalidResponse)
        }
    }
    async fn send_packet_async(&self, port: &mut serial2_tokio::SerialPort, packet: &[u8]) -> Result<(), YmodemError> {
        port.write_all(packet).await.unwrap();
        while let Err(e) = Self::wait_for_ack_async(port).await {
            if e == YmodemError::RequestReSend {
                port.write_all(packet).await.unwrap();
            } else {
                return Err(e);
            }
        }
        Ok(())
    }
    pub async fn send_async(&self, port: &mut serial2_tokio::SerialPort) -> Result<(), YmodemError> {
        let mut response = [0; 1];
        loop {
            port.read_exact(&mut response).await;
            if response[0] == YmodemControlCode::C as u8 {
                break;
            }
        }
        let file_header = self.create_file_header();
        self.send_packet_async(port, &file_header).await?;
        if Self::wait_msg_async(port).await != YmodemControlCode::C as u8 {
          return Err(YmodemError::InvalidResponse)
        }
        for (block_number, chunk) in self.fdata.chunks(PACKET_SIZE).enumerate() {
            let data_block = Self::create_data_block(chunk, (block_number + 1) as u8);
            self.send_packet_async(port, &data_block).await?;
        }
        // EOTの送信
        self.send_packet_async(port, &[YmodemControlCode::Eot as u8]).await?;
        if Self::wait_msg_async(port).await != YmodemControlCode::C as u8 {
          return Err(YmodemError::InvalidResponse)
        }
        let data_block = Self::create_data_block(&[0; PACKET_SIZE], 0);
        self.send_packet_async(port, &data_block).await?;
        // 最後のACKを待つ
        println!("YMODEM PASS!");
        Ok(())
    }
}

fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if (crc & 0x8000) != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}
