use bytes::BufMut;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::net::UdpSocket;

use crate::server_error::ServerError;
use tl_common::Result;

pub(crate) enum RemoteClient {
    Tcp(RemoteTcpClient),
    Udp(RemoteUdpClient),
}

///////////////////////////////////////////////////////////////////////////////
pub(crate) struct RemoteTcpClient {
    conn: TcpStream,
}

impl RemoteTcpClient {
    pub async fn build(addr: &str) -> Result<RemoteTcpClient> {
        let conn = TcpStream::connect(addr).await?;
        Ok(RemoteTcpClient { conn: conn })
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
}

///////////////////////////////////////////////////////////////////////////////
pub(crate) struct RemoteUdpClient {
    conn: UdpSocket,
}

impl RemoteUdpClient {
    pub async fn build() -> Result<RemoteUdpClient> {
        let conn = UdpSocket::bind("0.0.0.0:0").await?;
        Ok(RemoteUdpClient { conn: conn })
    }

    pub async fn readable(&mut self) -> Result<()> {
        self.conn.readable().await?;
        Ok(())
    }

    pub fn try_read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let (data_len, src_addr) = self.conn.try_recv_from(buf)?;
        let data = buf[..data_len].to_vec();
        let addr: String = src_addr.to_string();

        let n = 2 + addr.len() + 2 + data_len;
        let mut new_buf: Vec<u8> = Vec::with_capacity(n);
        new_buf.put_u16(addr.len() as u16);
        new_buf.put(addr.as_bytes());
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
            remote_udp_read_buf(buf, &mut buf_left_bytes, &mut buf_index, 2)?;
        let addr_len: usize = (((b[0] as u16) << 8) + b[1] as u16).into();
        if addr_len > 260 {
            return Err(Box::new(ServerError::TlRequestAddrInvalid));
        }
        // get addr
        let b = remote_udp_read_buf(
            buf,
            &mut buf_left_bytes,
            &mut buf_index,
            addr_len,
        )?;
        let addr = String::from_utf8(b.to_vec())?;
        // get data_len
        let b =
            remote_udp_read_buf(buf, &mut buf_left_bytes, &mut buf_index, 2)?;
        let data_len: usize = (((b[0] as u16) << 8) + b[1] as u16).into();
        // get data
        let data = remote_udp_read_buf(
            buf,
            &mut buf_left_bytes,
            &mut buf_index,
            data_len,
        )?;

        self.conn.send_to(&data[..], &addr).await?;

        Ok(())
    }
}

fn remote_udp_read_buf<'a>(
    buf: &'a [u8],
    buf_left_bytes: &mut usize,
    buf_index: &mut usize,
    n: usize,
) -> Result<&'a [u8]> {
    if *buf_left_bytes < n {
        return Err(Box::new(ServerError::TlUdpDataInvalid));
    }

    let b = &buf[*buf_index..(*buf_index + n)];
    *buf_left_bytes -= n;
    *buf_index += n;

    Ok(b)
}
