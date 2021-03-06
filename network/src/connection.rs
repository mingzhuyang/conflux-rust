// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use crate::{
    io::{IoContext, StreamToken},
    throttling::THROTTLING_SERVICE,
};
use bytes::Bytes;
use mio::{deprecated::*, tcp::*, *};
use std::{
    io::{self, Read, Write},
    marker::PhantomData,
    sync::atomic::{AtomicBool, Ordering as AtomicOrdering},
};

use priority_send_queue::{PrioritySendQueue, SendQueuePriority};

use crate::Error;
use std::sync::atomic::AtomicUsize;

#[derive(PartialEq, Eq)]
pub enum WriteStatus {
    Ongoing,
    LowPriority,
    Complete,
}

pub const MAX_PAYLOAD_SIZE: usize = (1 << 24) - 1;

static HIGH_PRIORITY_PACKETS: AtomicUsize = AtomicUsize::new(0);

fn incr_high_priority_packets() {
    assert_ne!(
        HIGH_PRIORITY_PACKETS.fetch_add(1, AtomicOrdering::SeqCst),
        std::usize::MAX
    );
}

fn decr_high_priority_packets() {
    assert_ne!(
        HIGH_PRIORITY_PACKETS.fetch_sub(1, AtomicOrdering::SeqCst),
        0
    );
}

fn has_high_priority_packets() -> bool {
    HIGH_PRIORITY_PACKETS.load(AtomicOrdering::SeqCst) > 0
}

pub trait GenericSocket: Read + Write {}

impl GenericSocket for TcpStream {}

pub trait PacketSizer {
    fn packet_size(_: &Bytes) -> usize;
}

/// This information is to measure the congestion situation of network.
#[allow(dead_code)]
pub struct SendQueueStatus {
    queue_length: usize,
}

pub struct GenericConnection<Socket: GenericSocket, Sizer: PacketSizer> {
    token: StreamToken,
    socket: Socket,
    recv_buf: Bytes,
    send_queue: PrioritySendQueue<(Vec<u8>, usize)>,
    interest: Ready,
    registered: AtomicBool,
    phantom: PhantomData<Sizer>,
}

impl<Socket: GenericSocket, Sizer: PacketSizer> Drop
    for GenericConnection<Socket, Sizer>
{
    fn drop(&mut self) {
        let mut service = THROTTLING_SERVICE.write();
        while let Some(((packet, pos), priority)) = self.send_queue.pop_front()
        {
            if pos < packet.len() {
                service.on_dequeue(packet.len() - pos);
                if priority == SendQueuePriority::High {
                    decr_high_priority_packets();
                }
            }
        }
    }
}

impl<Socket: GenericSocket, Sizer: PacketSizer>
    GenericConnection<Socket, Sizer>
{
    pub fn readable(&mut self) -> io::Result<Option<Bytes>> {
        let mut buf: [u8; 1024] = [0; 1024];
        loop {
            match self.socket.read(&mut buf) {
                Ok(size) => {
                    trace!(
                        "Succeed to read socket data, token = {}, size = {}",
                        self.token,
                        size
                    );
                    if size == 0 {
                        break;
                    }
                    self.recv_buf.extend_from_slice(&buf[0..size]);
                }
                Err(e) => {
                    if e.kind() != io::ErrorKind::WouldBlock {
                        debug!("Failed to read socket data, token = {}, err = {:?}", self.token, e);
                        return Err(e);
                    }
                    break;
                }
            }
        }

        let size = Sizer::packet_size(&self.recv_buf);
        if size == 0 {
            Ok(None)
        } else {
            trace!("Packet received, token = {}, size = {}", self.token, size);
            Ok(Some(self.recv_buf.split_to(size)))
        }
    }

    fn write(&mut self) -> Result<WriteStatus, Error> {
        if self.send_queue.is_send_queue_empty(SendQueuePriority::High)
            && has_high_priority_packets()
        {
            trace!(
                "Give up to send socket data due to low priority, token = {}",
                self.token
            );
            return Ok(WriteStatus::LowPriority);
        }

        let buf = match self.send_queue.front_mut() {
            Some((buf, promoted)) => {
                if promoted {
                    trace!("Low priority packet promoted to high priority, token = {}", self.token);
                    incr_high_priority_packets();
                }
                buf
            }
            None => return Ok(WriteStatus::Complete),
        };
        let len = buf.0.len();
        let pos = buf.1;
        if pos >= len {
            error!(
                "Unexpected connection data, token = {}, len = {}, pos = {}",
                self.token, len, pos
            );
            return Ok(WriteStatus::Complete);
        }

        let size = self.socket.write(&buf.0[pos..])?;

        trace!(
            "Succeed to send socket data, token = {}, size = {}",
            self.token,
            size
        );
        THROTTLING_SERVICE.write().on_dequeue(size);

        if pos + size < len {
            buf.1 += size;
            Ok(WriteStatus::Ongoing)
        } else {
            trace!("Packet sent, token = {}, size = {}", self.token, len);
            decr_high_priority_packets();
            Ok(WriteStatus::Complete)
        }
    }

    pub fn writable<Message: Sync + Send + Clone + 'static>(
        &mut self, io: &IoContext<Message>,
    ) -> Result<WriteStatus, Error> {
        let status = self.write();

        if let Ok(WriteStatus::Complete) = status {
            self.send_queue.pop_front();
        }

        if self.send_queue.is_empty() {
            self.interest.remove(Ready::writable());
        }

        io.update_registration(self.token)?;

        status
    }

    pub fn send<Message: Sync + Send + Clone + 'static>(
        &mut self, io: &IoContext<Message>, data: &[u8],
        priority: SendQueuePriority,
    ) -> Result<SendQueueStatus, Error>
    {
        if !data.is_empty() {
            trace!(
                "Sending packet, token = {}, size = {}",
                self.token,
                data.len()
            );
            THROTTLING_SERVICE.write().on_enqueue(data.len())?;
            let message = data.to_vec();
            self.send_queue.push_back((message, 0), priority);
            if priority == SendQueuePriority::High {
                incr_high_priority_packets();
            }
            if !self.interest.is_writable() {
                self.interest.insert(Ready::writable());
            }
            io.update_registration(self.token).ok();
        }

        Ok(SendQueueStatus {
            queue_length: self.send_queue.len(),
        })
    }

    pub fn is_sending(&self) -> bool { self.interest.is_writable() }
}

pub type Connection<Sizer> = GenericConnection<TcpStream, Sizer>;

impl<Sizer: PacketSizer> Connection<Sizer> {
    pub fn new(token: StreamToken, socket: TcpStream) -> Self {
        Connection {
            token: token,
            socket: socket,
            recv_buf: Bytes::new(),
            send_queue: PrioritySendQueue::new(),
            interest: Ready::hup() | Ready::readable(),
            registered: AtomicBool::new(false),
            phantom: PhantomData,
        }
    }

    pub fn register_socket<H: Handler>(
        &self, reg: Token, event_loop: &mut EventLoop<H>,
    ) -> io::Result<()> {
        if self.registered.load(AtomicOrdering::SeqCst) {
            return Ok(());
        }
        trace!(
            "Connection register, token = {}, reg = {:?}",
            self.token,
            reg
        );
        if let Err(e) = event_loop.register(
            &self.socket,
            reg,
            self.interest,
            PollOpt::edge(),
        ) {
            trace!(
                "Failed to register socket, token = {}, reg = {:?}, err = {:?}",
                self.token,
                reg,
                e
            );
        }
        self.registered.store(true, AtomicOrdering::SeqCst);
        Ok(())
    }

    pub fn update_socket<H: Handler>(
        &self, reg: Token, event_loop: &mut EventLoop<H>,
    ) -> io::Result<()> {
        trace!(
            "Connection reregister, token = {}, reg = {:?}",
            self.token,
            reg
        );
        if !self.registered.load(AtomicOrdering::SeqCst) {
            self.register_socket(reg, event_loop)
        } else {
            event_loop
                .reregister(&self.socket, reg, self.interest, PollOpt::edge())
                .unwrap_or_else(|e| {
                    trace!("Failed to reregister socket, token = {}, reg = {:?}, err = {:?}", self.token, reg, e);
                });
            Ok(())
        }
    }

    pub fn deregister_socket<H: Handler>(
        &self, event_loop: &mut EventLoop<H>,
    ) -> io::Result<()> {
        trace!("Connection deregister, token = {}", self.token);
        event_loop.deregister(&self.socket).ok();
        Ok(())
    }

    pub fn token(&self) -> StreamToken { self.token }
}

#[cfg(test)]
mod tests {
    use std::{
        cmp,
        io::{Read, Result, Write},
    };

    use super::*;
    use crate::io::*;
    use bytes::{Buf, Bytes, IntoBuf};
    use mio::Ready;

    struct TestSocket {
        read_buf: Vec<u8>,
        write_buf: Vec<u8>,
        cursor: usize,
        buf_size: usize,
    }

    impl TestSocket {
        fn new() -> Self {
            TestSocket {
                read_buf: vec![],
                write_buf: vec![],
                cursor: 0,
                buf_size: 0,
            }
        }

        fn with_buf(buf_size: usize) -> Self {
            TestSocket {
                read_buf: vec![],
                write_buf: vec![],
                cursor: 0,
                buf_size: buf_size,
            }
        }
    }

    impl Read for TestSocket {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            let end = cmp::min(self.read_buf.len(), self.cursor + buf.len());
            if self.cursor > end {
                return Ok(0);
            }
            let len = end - self.cursor;
            if len == 0 {
                Ok(0)
            } else {
                for i in self.cursor..end {
                    buf[i - self.cursor] = self.read_buf[i];
                }
                self.cursor = end;
                Ok(len)
            }
        }
    }

    impl Write for TestSocket {
        fn write(&mut self, buf: &[u8]) -> Result<usize> {
            if self.buf_size == 0 || buf.len() < self.buf_size {
                self.write_buf.extend(buf.iter().cloned());
                Ok(buf.len())
            } else {
                self.write_buf
                    .extend(buf.iter().take(self.buf_size).cloned());
                Ok(self.buf_size)
            }
        }

        fn flush(&mut self) -> Result<()> {
            unimplemented!();
        }
    }

    impl GenericSocket for TestSocket {}

    struct TestPacketSizer;

    impl PacketSizer for TestPacketSizer {
        fn packet_size(raw_packet: &Bytes) -> usize {
            let buf = &raw_packet.into_buf() as &Buf;
            if buf.remaining() >= 1 {
                let size = buf.bytes()[0] as usize;
                if buf.remaining() >= size {
                    size
                } else {
                    0
                }
            } else {
                0
            }
        }
    }

    type TestConnection = GenericConnection<TestSocket, TestPacketSizer>;

    impl TestConnection {
        fn new() -> Self {
            TestConnection {
                token: 1234567890usize,
                socket: TestSocket::new(),
                send_queue: PrioritySendQueue::new(),
                recv_buf: Bytes::new(),
                interest: Ready::hup() | Ready::readable(),
                registered: AtomicBool::new(false),
                phantom: PhantomData,
            }
        }
    }

    fn test_io() -> IoContext<i32> {
        IoContext::new(IoChannel::disconnected(), 0)
    }

    #[test]
    fn connection_write_empty() {
        let mut connection = TestConnection::new();
        let status = connection.writable(&test_io());
        assert!(status.is_ok());
        assert!(WriteStatus::Complete == status.unwrap());
    }

    #[test]
    fn connection_write_is_buffered() {
        let mut connection = TestConnection::new();
        connection.socket = TestSocket::with_buf(1024);
        let data = (vec![0; 10240], 0);
        connection
            .send_queue
            .push_back(data, SendQueuePriority::High);
        incr_high_priority_packets();

        let status = connection.writable(&test_io());

        assert!(status.is_ok());
        assert_eq!(1, connection.send_queue.len());
    }

    #[test]
    fn connection_read() {
        let mut connection = TestConnection::new();

        connection.socket.read_buf = vec![3, 0];
        {
            let status = connection.readable();
            assert!(status.is_ok());
            assert!(status.unwrap().is_none());
        }

        connection.socket.read_buf.extend_from_slice(&[0u8]);
        {
            let status = connection.readable();
            assert!(status.is_ok());
            assert_eq!(status.unwrap().unwrap().len(), 3);
        }

        {
            let status = connection.readable();
            assert!(status.is_ok());
            assert!(status.unwrap().is_none());
        }
    }
}
