use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::time::Duration;

use crate::Config;
use crate::remote_client::RemoteClient;
use crate::remote_client::RemoteTcpClient;
use crate::remote_client::RemoteUdpClient;
use crate::tl_server::TlServer;
use tl_common::Result;

pub(crate) async fn handle_proxy(config: &'static Config) -> Result<()> {
    let listener = TcpListener::bind(&config.local_addr).await?;

    loop {
        match listener.accept().await {
            Ok((conn, addr)) => {
                tokio::spawn(proxy(conn, config));
            }
            Err(e) => {
                eprintln!("TcpListener::accept() failed: {}", e);
                tokio::time::sleep(Duration::from_millis(1000)).await;
                tokio::task::yield_now().await;
            }
        }
    }
}

async fn proxy(client_conn: TcpStream, config: &'static Config) -> Result<()> {
    let mut s = TlServer::new(
        client_conn,
        &config.sec_key,
        config.fake_request.as_bytes(),
        config.fake_response.as_bytes(),
    );

    let c = s.accept().await?;
    match c {
        RemoteClient::Tcp(c) => proxy_tcp(c, s).await,
        RemoteClient::Udp(c) => proxy_udp(c, s).await,
    }
}

///////////////////////////////////////////////////////////////////////////////
async fn proxy_tcp(mut c: RemoteTcpClient, mut s: TlServer) -> Result<()> {
    let mut copy_buf: Vec<u8> = vec![0; 32 * 1024];
    loop {
        tokio::select! {
            _ = c.readable() => {
                let ret = tcp_copy_data_c2s(
                    &mut c, &mut s, copy_buf.as_mut_slice()).await?;
                if ret == false {
                    break;
                }
            }
            _ = s.readable() => {
                let ret = tcp_copy_data_s2c(
                    &mut s, &mut c, copy_buf.as_mut_slice()).await?;
                if ret == false {
                    break;
                }
            }
        };
    }

    Ok(())
}

async fn tcp_copy_data_c2s(
    c: &mut RemoteTcpClient,
    s: &mut TlServer,
    buf: &mut [u8],
) -> Result<bool> {
    loop {
        match c.try_read(buf) {
            Ok(n) => {
                if n == 0 {
                    return Ok(false);
                }
                s.write_all(&buf[..n]).await?;
            }
            Err(e) => {
                if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
                    if io_error.kind() == std::io::ErrorKind::WouldBlock {
                        return Ok(true);
                    }
                }
                return Err(e.into());
            }
        }
    }
}

async fn tcp_copy_data_s2c(
    s: &mut TlServer,
    c: &mut RemoteTcpClient,
    buf: &mut [u8],
) -> Result<bool> {
    loop {
        match s.try_read(buf) {
            Ok(n) => {
                if n == 0 {
                    return Ok(false);
                }
                c.write_all(&buf[..n]).await?;
            }
            Err(e) => {
                if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
                    if io_error.kind() == std::io::ErrorKind::WouldBlock {
                        return Ok(true);
                    }
                }
                return Err(e.into());
            }
        }
    }
}

///////////////////////////////////////////////////////////////////////////////
async fn proxy_udp(mut c: RemoteUdpClient, mut s: TlServer) -> Result<()> {
    let mut copy_buf: Vec<u8> = vec![0; 32 * 1024];
    loop {
        tokio::select! {
            _ = c.readable() => {
                udp_copy_data_c2s(
                    &mut c, &mut s, copy_buf.as_mut_slice()).await?;
            }
            _ = s.readable() => {
                let ret = udp_copy_data_s2c(
                    &mut s, &mut c, copy_buf.as_mut_slice()).await?;
                if ret == false {
                    break;
                }
            }
        };
    }

    Ok(())
}

async fn udp_copy_data_c2s(
    c: &mut RemoteUdpClient,
    s: &mut TlServer,
    buf: &mut [u8],
) -> Result<()> {
    loop {
        match c.try_read(buf) {
            Ok(n) => {
                if n == 0 {
                    return Ok(());
                }
                s.write_all(&buf[..n]).await?;
            }
            Err(e) => {
                if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
                    if io_error.kind() == std::io::ErrorKind::WouldBlock {
                        return Ok(());
                    }
                }
                return Err(e.into());
            }
        }
    }
}

async fn udp_copy_data_s2c(
    s: &mut TlServer,
    c: &mut RemoteUdpClient,
    buf: &mut [u8],
) -> Result<bool> {
    loop {
        match s.try_read(buf) {
            Ok(n) => {
                if n == 0 {
                    return Ok(false);
                }
                c.write_all(&buf[..n]).await?;
            }
            Err(e) => {
                if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
                    if io_error.kind() == std::io::ErrorKind::WouldBlock {
                        return Ok(true);
                    }
                }
                return Err(e.into());
            }
        }
    }
}
