// Licensed under the Apache-2.0 license

use std::{
    io::{self, ErrorKind},
    time::{Duration, Instant},
};

use libftdi1_sys::{ftdi_bits_type, ftdi_interface, ftdi_parity_type, ftdi_stopbits_type};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{
    ftdi::{BitMode, FtdiCtx},
    usb_port_path::UsbPortPath,
};

pub struct FtdiUartReader {
    ftdi: Rc<RefCell<FtdiCtx>>,
}

pub struct FtdiUartReaderBlocking {
    ftdi: Rc<RefCell<FtdiCtx>>,
    timeout: Option<Duration>,
}

pub struct FtdiUartWriter {
    ftdi: Rc<RefCell<FtdiCtx>>,
}
impl FtdiUartWriter {
    pub fn send_break(&self) -> anyhow::Result<()> {
        const BREAK_TIME: Duration = Duration::from_micros(100);

        let mut ftdi = self.ftdi.borrow_mut();
        ftdi.set_bitmode(0x01, BitMode::BitBang)?;
        ftdi.write_all_data(&[0x00])?;
        std::thread::sleep(BREAK_TIME);
        ftdi.set_bitmode(0x01, BitMode::Reset)?;
        ftdi.set_baudrate(115200)?;
        ftdi.set_line_property(
            ftdi_bits_type::BITS_8,
            ftdi_stopbits_type::STOP_BIT_1,
            ftdi_parity_type::NONE,
        )?;
        Ok(())
    }
}

pub fn open(
    port_path: UsbPortPath,
    iface: ftdi_interface,
) -> anyhow::Result<(FtdiUartReader, FtdiUartWriter)> {
    let mut ftdi = FtdiCtx::open(port_path, iface)?;
    ftdi.set_bitmode(0, BitMode::Reset)?;
    ftdi.set_baudrate(115200)?;
    ftdi.set_line_property(
        ftdi_bits_type::BITS_8,
        ftdi_stopbits_type::STOP_BIT_1,
        ftdi_parity_type::NONE,
    )?;
    let ftdi = Rc::new(RefCell::new(ftdi));
    Ok((
        FtdiUartReader { ftdi: ftdi.clone() },
        FtdiUartWriter { ftdi },
    ))
}

pub fn open_blocking(
    port_path: UsbPortPath,
    iface: ftdi_interface,
    timeout: Option<Duration>,
) -> anyhow::Result<(FtdiUartReaderBlocking, FtdiUartWriter)> {
    let (rx, tx) = open(port_path, iface)?;
    Ok((
        FtdiUartReaderBlocking {
            ftdi: rx.ftdi,
            timeout,
        },
        tx,
    ))
}

impl io::Read for FtdiUartReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.ftdi
            .borrow_mut()
            .read_data(buf)
            .map_err(|e| io::Error::new(ErrorKind::Other, e))
    }
}

impl io::Read for FtdiUartReaderBlocking {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let start_time = Instant::now();
        loop {
            let bytes_read = self
                .ftdi
                .borrow_mut()
                .read_data(buf)
                .map_err(|e| io::Error::new(ErrorKind::Other, e))?;
            if bytes_read > 0 {
                return Ok(bytes_read);
            }
            if let Some(timeout) = self.timeout {
                if start_time.elapsed() >= timeout {
                    return Err(io::Error::new(
                        ErrorKind::TimedOut,
                        "timed out reading from FPGA UART",
                    ));
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum UartEvent {
    Line(String),
    TickTimeout,
    Eof,
}

/// Line-buffered UART reader that preserves partial lines across `ErrorKind::TimedOut` ticks.
pub struct UartLineStream<R> {
    reader: R,
    pending: Vec<u8>,
}

const MAX_PENDING_LINE_BYTES: usize = 64 * 1024;

impl<R: io::Read> UartLineStream<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            pending: Vec::new(),
        }
    }

    fn take_complete_line(&mut self) -> Option<String> {
        let pos = self.pending.iter().position(|&b| b == b'\n')?;
        let mut line_bytes: Vec<u8> = self.pending.drain(..=pos).collect();
        while matches!(line_bytes.last(), Some(b'\n' | b'\r')) {
            line_bytes.pop();
        }
        Some(String::from_utf8_lossy(&line_bytes).into_owned())
    }

    pub fn next_event(&mut self) -> io::Result<UartEvent> {
        if let Some(line) = self.take_complete_line() {
            return Ok(UartEvent::Line(line));
        }

        let mut chunk = [0_u8; 4096];
        loop {
            match self.reader.read(&mut chunk) {
                Ok(0) => {
                    if self.pending.is_empty() {
                        return Ok(UartEvent::Eof);
                    }
                    let mut line_bytes = std::mem::take(&mut self.pending);
                    while matches!(line_bytes.last(), Some(b'\n' | b'\r')) {
                        line_bytes.pop();
                    }
                    return Ok(UartEvent::Line(
                        String::from_utf8_lossy(&line_bytes).into_owned(),
                    ));
                }
                Ok(n) => {
                    self.pending.extend_from_slice(&chunk[..n]);
                    if let Some(line) = self.take_complete_line() {
                        return Ok(UartEvent::Line(line));
                    }
                    if self.pending.len() >= MAX_PENDING_LINE_BYTES {
                        let line_bytes = std::mem::take(&mut self.pending);
                        return Ok(UartEvent::Line(
                            String::from_utf8_lossy(&line_bytes).into_owned(),
                        ));
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                    return Ok(UartEvent::TickTimeout);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl io::Write for FtdiUartWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.ftdi
            .borrow_mut()
            .write_data(buf)
            .map_err(|e| io::Error::new(ErrorKind::Other, e))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    enum MockStep {
        Data(&'static [u8]),
        Timeout,
        Eof,
    }

    struct MockReader {
        steps: VecDeque<MockStep>,
    }

    impl io::Read for MockReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.steps.pop_front().unwrap_or(MockStep::Eof) {
                MockStep::Data(bytes) => {
                    let n = bytes.len().min(buf.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                    Ok(n)
                }
                MockStep::Timeout => Err(io::Error::new(ErrorKind::TimedOut, "tick")),
                MockStep::Eof => Ok(0),
            }
        }
    }

    #[test]
    fn test_uart_line_stream_preserves_partial_lines_across_timeouts() {
        let reader = MockReader {
            steps: VecDeque::from([
                MockStep::Data(b"CALIPTRA_FPGA_INFO {\"ip\":\"10.0.0.5\""),
                MockStep::Timeout,
                MockStep::Data(b",\"health_port\":8080,\"variant\":\"core\"}\r\nsecond"),
                MockStep::Data(b" line\nthird"),
                MockStep::Eof,
            ]),
        };

        let mut stream = UartLineStream::new(reader);
        assert_eq!(stream.next_event().unwrap(), UartEvent::TickTimeout);
        assert_eq!(
            stream.next_event().unwrap(),
            UartEvent::Line(
                r#"CALIPTRA_FPGA_INFO {"ip":"10.0.0.5","health_port":8080,"variant":"core"}"#
                    .to_string()
            )
        );
        assert_eq!(
            stream.next_event().unwrap(),
            UartEvent::Line("second line".to_string())
        );
        assert_eq!(
            stream.next_event().unwrap(),
            UartEvent::Line("third".to_string())
        );
        assert_eq!(stream.next_event().unwrap(), UartEvent::Eof);
    }
}
