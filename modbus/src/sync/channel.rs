use tokio::sync::mpsc::{
    channel,
    error::{SendError, TryRecvError, TrySendError},
    Receiver as TokioReceiver, Sender as TokioSender,
};

pub struct DuplexChannel<S, R> {
    sender: TokioSender<S>,
    receiver: TokioReceiver<R>,
}

pub struct DuplexChannelPair<S, R> {
    pair: (DuplexChannel<S, R>, DuplexChannel<R, S>),
}

impl<S, R> DuplexChannelPair<S, R> {
    pub fn new(size: usize) -> Self {
        let (c1, r1) = channel::<S>(size);
        let (c2, r2) = channel::<R>(size);
        Self {
            pair: (
                DuplexChannel {
                    sender: c1,
                    receiver: r2,
                },
                DuplexChannel {
                    sender: c2,
                    receiver: r1,
                },
            ),
        }
    }

    pub fn split(self) -> (DuplexChannel<S, R>, DuplexChannel<R, S>) {
        self.pair
    }
}

impl<S, R> DuplexChannel<S, R> {
    pub async fn send(&self, value: S) -> Result<(), SendError<S>> {
        self.sender.send(value).await
    }

    pub fn try_send(&self, value: S) -> Result<(), TrySendError<S>> {
        self.sender.try_send(value)
    }

    pub async fn recv(&mut self) -> Option<R> {
        self.receiver.recv().await
    }

    pub fn try_recv(&mut self) -> Result<R, TryRecvError> {
        self.receiver.try_recv()
    }

    pub fn is_closed(&self) -> bool {
        self.sender.is_closed() || self.receiver.is_closed()
    }

    pub fn sender(&self) -> TokioSender<S> {
        self.sender.clone()
    }
}
