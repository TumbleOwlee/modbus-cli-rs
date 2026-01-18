use std::hash::Hash;

use crate::mem::memory::Memory;
use crate::mem::range::Range;
use crate::mem::{Request, Response};
use crate::sync::channel::{DuplexChannel, DuplexChannelPair};

pub struct Handler<K>
where
    K: Hash + Eq + Clone + Default,
{
    memory: Memory<K>,
    channels: Vec<DuplexChannel<Response, Request<K>>>,
}

impl<K> Handler<K>
where
    K: Hash + Eq + Clone + Default,
{
    pub fn init(slices: &[(K, &[Range])]) -> Result<Self, anyhow::Error> {
        let mut memory = Memory::<K>::default();
        for (id, ranges) in slices.iter() {
            memory.add_ranges(id.clone(), ranges)?;
        }
        Ok(Self {
            memory,
            channels: vec![],
        })
    }

    pub fn get_channel(&mut self, size: usize) -> DuplexChannel<Request<K>, Response> {
        let (c1, c2) = DuplexChannelPair::new(size).split();
        self.channels.push(c2);
        c1
    }

    pub fn attach_channel(&mut self, channel: DuplexChannel<Response, Request<K>>) -> () {
        self.channels.push(channel);
    }

    pub async fn run(mut self) {
        loop {
            self.channels = self
                .channels
                .into_iter()
                .filter(|c| c.is_closed())
                .collect();

            for channel in self.channels.iter_mut() {
                let result = channel.try_recv();
                if result.is_err() {
                    continue;
                }

                match result.unwrap() {
                    Request::Shutdown => {
                        return;
                    }
                    Request::Read((id, range)) => match self.memory.read(id, &range) {
                        Ok(values) => {
                            channel
                                .send(Response::Values(values.iter().cloned().collect()))
                                .await;
                        }
                        Err(e) => {
                            channel.send(Response::Error(e));
                        }
                    },
                    Request::Write((id, range, values)) => {
                        match self.memory.write(id, &range, &values) {
                            Ok(_) => {
                                channel.send(Response::Confirm).await;
                            }
                            Err(e) => {
                                channel.send(Response::Error(e));
                            }
                        }
                    }
                    Request::NoOperation => {}
                }
            }
        }
    }
}
