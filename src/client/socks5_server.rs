use bytes::BufMut;
use std::net::IpAddr;
use std::net::SocketAddr;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::net::UdpSocket;

use crate::client_error::ClientError;
use tl_common::Result;

pub(crate) enum Socks5CmdType {
    Connect,
    UdpAssociate,
}

///////////////////////////////////////////////////////////////////////////////
pub(crate) struct Socks5Request {
    pub cmd: Socks5CmdType,
    pub dst_addr: String,
}

///////////////////////////////////////////////////////////////////////////////
pub(crate) struct Socks5Server {
    conn: TcpStream,
}

impl Socks5Server {
    pub fn new(conn: TcpStream) -> Socks5Server {
        Socks5Server { conn: conn }
    }

    pub async fn readable(&mut self) -> Result<()> {
        self.conn.readable().await?;
        Ok(())
    }

    pub fn try_read(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.conn.try_read(buf)?)
    }

    pub async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.conn.write_all(buf).await?;
        Ok(())
    }

    pub async fn method_select(&mut self) -> Result<()> {
        let mut buf: Vec<u8> = vec![0; 256];

        let b = &mut buf[..2];
        self.conn.read_exact(b).await?;

        let version = b[0];
        let methods_bytes = b[1];

        // check version
        if version != 0x05 {
            return Err(Box::new(ClientError::Socks5VersionInvalid));
        }

        // discard methods
        let b = &mut buf[..methods_bytes.into()];
        self.conn.read_exact(b).await?;

        // answer server accepted method
        self.conn.write_all(&[0x05, 0x00]).await?;

        Ok(())
    }

    pub async fn receive_request(&mut self) -> Result<Socks5Request> {
        let mut buf: Vec<u8> = vec![0; 256];

        let b = &mut buf[..4];
        self.conn.read_exact(b).await?;

        let version = b[0];
        let cmd = b[1];
        let addr_type = b[3];

        // check version
        if version != 0x05 {
            return Err(Box::new(ClientError::Socks5VersionInvalid));
        }
        // get cmd
        let socks5_cmd = match cmd {
            0x01 => Socks5CmdType::Connect,
            0x03 => Socks5CmdType::UdpAssociate,
            _ => return Err(Box::new(ClientError::Socks5CmdNotSupported)),
        };
        // get dst addr
        let socks5_dst_addr = match addr_type {
            0x01 => {
                // ipv4
                let b = &mut buf[..6];
                self.conn.read_exact(b).await?;
                socks5_convert_ipv4_addr(b)
            }
            0x03 => {
                // domain
                let b = &mut buf[..1];
                self.conn.read_exact(b).await?;
                let domain_length = b[0] as usize;

                let b = &mut buf[..(domain_length + 2)];
                self.conn.read_exact(b).await?;
                socks5_convert_domain_addr(b)?
            }
            _ => {
                return Err(Box::new(ClientError::Socks5AddrTypeNotSupported));
            }
        };

        Ok(Socks5Request {
            cmd: socks5_cmd,
            dst_addr: socks5_dst_addr,
        })
    }

    pub async fn notify_connect_success(&mut self) -> Result<()> {
        self.conn
            .write_all(&[
                0x05, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ])
            .await?;

        Ok(())
    }

    pub async fn notify_udp_associate_success(
        &mut self,
        port: u16,
    ) -> Result<()> {
        let b0 = port as u8;
        let b1 = (port >> 8) as u8;

        self.conn
            .write_all(&[
                0x05, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, b1, b0,
            ])
            .await?;
        Ok(())
    }
}

///////////////////////////////////////////////////////////////////////////////
pub(crate) struct Socks5UdpServer {
    conn: UdpSocket,
    connected: bool,
}

impl Socks5UdpServer {
    pub async fn build(host: &str) -> Result<Socks5UdpServer> {
        let conn = UdpSocket::bind(host).await?;
        Ok(Socks5UdpServer {
            conn: conn,
            connected: false,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.conn.local_addr()?)
    }

    pub async fn readable(&mut self) -> Result<()> {
        self.conn.readable().await?;
        Ok(())
    }

    pub async fn try_read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let (mut buf_left_bytes, src_addr) = self.conn.try_recv_from(buf)?;
        let mut buf_index: usize = 0;

        if self.connected == false {
            self.conn.connect(src_addr).await?;
            self.connected = true;
        }

        let b =
            socks5_udp_read_buf(buf, &mut buf_left_bytes, &mut buf_index, 4)?;
        let frag = b[2];
        let addr_type = b[3];

        // check frag
        if frag != 0x00 {
            // drop frag udp package
            return Ok(0);
        }
        // get dst addr
        let dst_addr = match addr_type {
            0x01 => {
                let b = socks5_udp_read_buf(
                    buf,
                    &mut buf_left_bytes,
                    &mut buf_index,
                    6,
                )?;
                socks5_convert_ipv4_addr(b)
            }
            _ => {
                return Err(Box::new(ClientError::Socks5AddrTypeNotSupported));
            }
        };

        // get data
        if buf_left_bytes == 0 {
            // drop
            return Ok(0);
        }
        let data = buf[buf_index..(buf_index + buf_left_bytes)].to_vec();

        println!(
            "proxy_udp_request: [{}] => [{}] ({})",
            src_addr,
            dst_addr,
            data.len()
        );

        let n = 2 + dst_addr.len() + 2 + data.len();
        let mut new_buf: Vec<u8> = Vec::with_capacity(n);
        new_buf.put_u16(dst_addr.len() as u16);
        new_buf.put(dst_addr.as_bytes());
        new_buf.put_u16(data.len() as u16);
        new_buf.put(data.as_slice());
        buf[..n].copy_from_slice(new_buf.as_slice());

        Ok(n)
    }

    pub async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        let mut buf_left_bytes = buf.len();
        let mut buf_index: usize = 0;

        // get addr_len
        let b =
            socks5_udp_read_buf(buf, &mut buf_left_bytes, &mut buf_index, 2)?;
        let addr_len: usize = (((b[0] as u16) << 8) + b[1] as u16).into();
        // get addr
        let b = socks5_udp_read_buf(
            buf,
            &mut buf_left_bytes,
            &mut buf_index,
            addr_len,
        )?;
        let addr = String::from_utf8(b.to_vec())?;
        // get data_len
        let b =
            socks5_udp_read_buf(buf, &mut buf_left_bytes, &mut buf_index, 2)?;
        let data_len: usize = (((b[0] as u16) << 8) + b[1] as u16).into();
        // get data
        let data = socks5_udp_read_buf(
            buf,
            &mut buf_left_bytes,
            &mut buf_index,
            data_len,
        )?;
        // parse sock_addr
        let sock_addr: SocketAddr = addr.parse()?;

        let n = 2
            + 1
            + 1
            + match sock_addr {
                SocketAddr::V4(_) => 4,
                SocketAddr::V6(_) => 16,
            }
            + data_len;
        let mut new_buf: Vec<u8> = Vec::with_capacity(n);
        new_buf.put_u16(0);
        new_buf.put_u8(0x00);
        match sock_addr.ip() {
            IpAddr::V4(ipv4_addr) => {
                new_buf.put_u8(0x01);
                new_buf.put(&ipv4_addr.octets()[..]);
            }
            IpAddr::V6(ipv6_addr) => {
                new_buf.put_u8(0x02);
                new_buf.put(&ipv6_addr.octets()[..]);
            }
        }
        new_buf.put_u16(sock_addr.port());
        new_buf.put(data);

        self.conn.send(new_buf.as_slice()).await?;

        Ok(())
    }
}

///////////////////////////////////////////////////////////////////////////////
fn socks5_udp_read_buf<'a>(
    buf: &'a [u8],
    buf_left_bytes: &mut usize,
    buf_index: &mut usize,
    n: usize,
) -> Result<&'a [u8]> {
    if *buf_left_bytes < n {
        return Err(Box::new(ClientError::Socks5UdpDataInvalid));
    }

    let b = &buf[*buf_index..(*buf_index + n)];
    *buf_left_bytes -= n;
    *buf_index += n;

    Ok(b)
}

fn socks5_convert_ipv4_addr(b: &[u8]) -> String {
    let port: u16 = ((b[4] as u16) << 8) + b[5] as u16;
    format!("{}.{}.{}.{}:{}", b[0], b[1], b[2], b[3], port)
}

fn socks5_convert_domain_addr(b: &[u8]) -> Result<String> {
    let n = b.len();
    let domain = std::str::from_utf8(&b[..(n - 2)])?;
    let port: u16 = ((b[n - 2] as u16) << 8) + b[n - 1] as u16;
    Ok(format!("{}:{}", domain, port))
}
