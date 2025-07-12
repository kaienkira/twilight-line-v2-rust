use core::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

use crate::Config;
use crate::socks5_server::Socks5CmdType;
use crate::socks5_server::Socks5Request;
use crate::socks5_server::Socks5Server;
use crate::socks5_server::Socks5UdpServer;
use crate::tl_client::TlClient;
use tl_common::Result;
use tl_common::TlConnectionType;

pub(crate) async fn handle_proxy(config: &'static Config) -> Result<()> {
    let listener = TcpListener::bind(&config.local_addr).await?;

    loop {
        match listener.accept().await {
            Ok((conn, addr)) => {
                tokio::spawn(proxy(conn, addr, config));
            }
            Err(e) => {
                eprintln!("TcpListener::accept() failed: {}", e);
            }
        }
    }
}

async fn proxy(
    client_conn: TcpStream,
    client_addr: SocketAddr,
    config: &'static Config,
) -> Result<()> {
    let mut s = Socks5Server::new(client_conn);
    s.method_select().await?;
    let req = s.receive_request().await?;
    match req.cmd {
        Socks5CmdType::Connect => proxy_tcp(client_addr, config, s, req).await,
        Socks5CmdType::UdpAssociate => proxy_udp(client_addr, config, s).await,
    }
}

///////////////////////////////////////////////////////////////////////////////
async fn proxy_tcp(
    client_addr: SocketAddr,
    config: &'static Config,
    mut s: Socks5Server,
    req: Socks5Request,
) -> Result<()> {
    println!("proxy_tcp_request: [{}] => [{}]", client_addr, req.dst_addr);

    let server_conn: TcpStream;
    match TcpStream::connect(&config.server_addr).await {
        Ok(v) => server_conn = v,
        Err(e) => {
            eprintln!("connect tl-server failed: {}", e);
            return Err(e.into());
        }
    }

    let mut c = TlClient::new(
        server_conn,
        &config.sec_key,
        config.fake_request.as_bytes(),
        config.fake_response.as_bytes(),
    );
    c.connect(TlConnectionType::Tcp, &req.dst_addr).await?;

    s.notify_connect_success().await?;

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
    c: &mut TlClient,
    s: &mut Socks5Server,
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
                } else {
                    return Err(e.into());
                }
            }
        }
    }
}

async fn tcp_copy_data_s2c(
    s: &mut Socks5Server,
    c: &mut TlClient,
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
                } else {
                    return Err(e.into());
                }
            }
        }
    }
}

///////////////////////////////////////////////////////////////////////////////
async fn proxy_udp(
    client_addr: SocketAddr,
    config: &'static Config,
    mut s: Socks5Server,
) -> Result<()> {
    println!("start_udp_proxy: [{}]", client_addr);

    let mut u = Socks5UdpServer::build(&config.local_udp_addr).await?;
    let udp_port = u.local_addr()?.port();

    let server_conn: TcpStream;
    match TcpStream::connect(&config.server_addr).await {
        Ok(v) => server_conn = v,
        Err(e) => {
            eprintln!("connect tl-server failed: {}", e);
            return Err(e.into());
        }
    }

    let mut c = TlClient::new(
        server_conn,
        &config.sec_key,
        config.fake_request.as_bytes(),
        config.fake_response.as_bytes(),
    );
    c.connect(TlConnectionType::Udp, "").await?;

    s.notify_udp_associate_success(udp_port).await?;

    let mut copy_buf: Vec<u8> = vec![0; 32 * 1024];
    loop {
        tokio::select! {
            _ = c.readable() => {
                let ret = udp_copy_data_c2u(
                    &mut c, &mut u, copy_buf.as_mut_slice()).await?;
                if ret == false {
                    break;
                }
            }
            _ = s.readable() => {
                let ret = udp_read_tcp_ctl(
                    &mut s, copy_buf.as_mut_slice())?;
                if ret == false {
                    break;
                }
            }
            _ = u.readable() => {
                udp_copy_data_u2c(
                    &mut u, &mut c, copy_buf.as_mut_slice()).await?;
            }
        }
    }

    Ok(())
}

fn udp_read_tcp_ctl(s: &mut Socks5Server, buf: &mut [u8]) -> Result<bool> {
    loop {
        match s.try_read(buf) {
            Ok(n) => {
                if n == 0 {
                    return Ok(false);
                }
            }
            Err(e) => {
                if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
                    if io_error.kind() == std::io::ErrorKind::WouldBlock {
                        return Ok(true);
                    }
                } else {
                    return Err(e.into());
                }
            }
        }
    }
}

async fn udp_copy_data_c2u(
    c: &mut TlClient,
    u: &mut Socks5UdpServer,
    buf: &mut [u8],
) -> Result<bool> {
    loop {
        match c.try_read(buf) {
            Ok(n) => {
                if n == 0 {
                    return Ok(false);
                }
                u.write_all(&buf[..n]).await?;
            }
            Err(e) => {
                if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
                    if io_error.kind() == std::io::ErrorKind::WouldBlock {
                        return Ok(true);
                    }
                } else {
                    return Err(e.into());
                }
            }
        }
    }
}

async fn udp_copy_data_u2c(
    u: &mut Socks5UdpServer,
    c: &mut TlClient,
    buf: &mut [u8],
) -> Result<()> {
    loop {
        match u.try_read(buf).await {
            Ok(n) => {
                if n == 0 {
                    return Ok(());
                }
                c.write_all(&buf[..n]).await?;
            }
            Err(e) => {
                if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
                    if io_error.kind() == std::io::ErrorKind::WouldBlock {
                        return Ok(());
                    }
                } else {
                    return Err(e.into());
                }
            }
        }
    }
}
