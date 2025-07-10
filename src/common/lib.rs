pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum TlConnectionType {
    Tcp = 1,
    Udp = 2,
}

pub mod util;
