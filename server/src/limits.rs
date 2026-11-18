//! Límites por conexión frente a clientes lentos (low-and-slow) y acaparamiento.
//!
//! Los plazos de handshake, cabeceras y petición acotan cada fase, pero no cuántas conexiones
//! puede abrir un origen ni cuánto tarda en leer la respuesta. Este acceptor rechaza conexiones
//! por encima de un tope global y por IP, y cierra cualquier conexión que supere su vida máxima,
//! también cuando el cliente lee la respuesta con lentitud.

use std::{
    collections::HashMap,
    future::{Ready, ready},
    io,
    net::IpAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use axum_server::accept::Accept;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Sleep,
};

#[derive(Clone)]
pub struct ConnectionLimits {
    total: Arc<Semaphore>,
    per_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
    max_per_ip: usize,
    lifetime: Duration,
}

impl ConnectionLimits {
    #[must_use]
    pub fn new(max_total: usize, max_per_ip: usize, lifetime: Duration) -> Self {
        Self {
            total: Arc::new(Semaphore::new(max_total)),
            per_ip: Arc::default(),
            max_per_ip,
            lifetime,
        }
    }

    fn admit(&self, ip: IpAddr) -> io::Result<Admission> {
        let refused = |reason| io::Error::new(io::ErrorKind::ConnectionRefused, reason);
        let permit = Arc::clone(&self.total)
            .try_acquire_owned()
            .map_err(|_| refused("tope global de conexiones"))?;
        let mut per_ip = self
            .per_ip
            .lock()
            .map_err(|_| refused("registro de conexiones dañado"))?;
        let open = per_ip.entry(ip).or_default();
        if *open >= self.max_per_ip {
            return Err(refused("tope de conexiones por origen"));
        }
        *open += 1;
        Ok(Admission {
            _permit: permit,
            ip,
            per_ip: Arc::clone(&self.per_ip),
        })
    }
}

/// Plaza ocupada por una conexión; se libera al cerrarla.
struct Admission {
    _permit: OwnedSemaphorePermit,
    ip: IpAddr,
    per_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

impl Drop for Admission {
    fn drop(&mut self) {
        if let Ok(mut per_ip) = self.per_ip.lock()
            && let Some(open) = per_ip.get_mut(&self.ip)
        {
            *open -= 1;
            if *open == 0 {
                per_ip.remove(&self.ip);
            }
        }
    }
}

/// Conexión admitida con vida máxima; al vencer, toda lectura o escritura falla.
pub struct Limited {
    stream: TcpStream,
    deadline: Pin<Box<Sleep>>,
    _admission: Admission,
}

impl Limited {
    fn expired(&mut self, context: &mut Context<'_>) -> bool {
        // registrar el temporizador despierta la tarea aunque el cliente no envíe nada
        self.deadline.as_mut().poll(context).is_ready()
    }
}

fn timed_out() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "vida máxima de la conexión agotada",
    )
}

use std::future::Future as _;

impl AsyncRead for Limited {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.expired(context) {
            return Poll::Ready(Err(timed_out()));
        }
        Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for Limited {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.expired(context) {
            return Poll::Ready(Err(timed_out()));
        }
        Pin::new(&mut self.stream).poll_write(context, data)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.expired(context) {
            return Poll::Ready(Err(timed_out()));
        }
        Pin::new(&mut self.stream).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(context)
    }
}

impl<S> Accept<TcpStream, S> for ConnectionLimits {
    type Stream = Limited;
    type Service = S;
    type Future = Ready<io::Result<(Limited, S)>>;

    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        ready(
            stream
                .peer_addr()
                .and_then(|peer| self.admit(peer.ip()))
                .map(|admission| {
                    (
                        Limited {
                            stream,
                            deadline: Box::pin(tokio::time::sleep(self.lifetime)),
                            _admission: admission,
                        },
                        service,
                    )
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn origins_share_a_global_cap_and_each_has_its_own_and_slots_return_on_close() {
        let limits = ConnectionLimits::new(3, 2, Duration::from_secs(1));
        let first = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let second = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        let a = limits.admit(first).unwrap();
        let _b = limits.admit(first).unwrap();
        assert!(limits.admit(first).is_err(), "tope por origen");
        let _c = limits.admit(second).unwrap();
        assert!(limits.admit(second).is_err(), "tope global");
        drop(a);
        assert!(limits.admit(first).is_ok(), "la plaza vuelve al cerrar");
        assert_eq!(limits.per_ip.lock().unwrap().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_client_is_disconnected_when_its_lifetime_ends() {
        use tokio::io::AsyncReadExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let _client = TcpStream::connect(address).await.unwrap();
        let (stream, ()) = ConnectionLimits::new(4, 4, Duration::from_secs(30))
            .accept(listener.accept().await.unwrap().0, ())
            .await
            .unwrap();
        let mut stream = stream;
        // el cliente no envía nada: la lectura queda en espera hasta la vida máxima
        let error = stream.read(&mut [0; 8]).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }
}
