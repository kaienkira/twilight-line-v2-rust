use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::net::UdpSocket;

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
        Ok(self.conn.try_recv(buf)?)
    }

    pub async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        Ok(())
    }
}
