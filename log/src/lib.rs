pub struct Log<const LEN: usize, const SIZE: usize> {
    buffer: [[char; LEN]; SIZE],
    write: usize,
    read: usize,
}

impl<const LEN: usize, const SIZE: usize> Log<LEN, SIZE> {
    pub fn init() -> Self {
        let buffer = [['\0'; LEN]; SIZE];

        Self {
            buffer,
            write: 0,
            read: 0,
        }
    }

    pub fn write(&mut self, msg: &str) {
        for (dst, src) in self.buffer[self.write].iter_mut().zip(msg.chars()) {
            *dst = src;
        }
        let next = (self.write + 1) % SIZE;
        if next == self.read {
            self.read += 1;
        }
        self.write = next;
    }

    pub fn peak(&self) -> Option<String> {
        if self.read != self.write {
            Some(self.buffer[self.read].iter().collect::<String>())
        } else {
            None
        }
    }

    pub fn peak_n(&self, cnt: usize) -> Option<Vec<String>> {
        let mut read = self.read;
        let mut msgs = Vec::with_capacity(cnt);
        while read != self.write && msgs.len() < cnt {
            msgs.push(self.buffer[self.read].iter().collect::<String>());
            read = (read + 1) % SIZE;
        }
        if msgs.is_empty() { None } else { Some(msgs) }
    }

    pub fn take(&mut self) -> Option<String> {
        if self.read != self.write {
            let msg = self.buffer[self.read].iter().collect::<String>();
            self.read += 1;
            Some(msg)
        } else {
            None
        }
    }

    pub fn take_n(&mut self, cnt: usize) -> Option<Vec<String>> {
        let mut msgs = Vec::with_capacity(cnt);
        while self.read != self.write && msgs.len() < cnt {
            msgs.push(self.buffer[self.read].iter().collect::<String>());
            self.read = (self.read + 1) % SIZE;
        }
        if msgs.is_empty() { None } else { Some(msgs) }
    }
}

#[cfg(test)]
mod tests {
    use super::Log;

    #[test]
    fn it_log() {
        let mut log: Log<10, 5> = Log::init();
        log.write("Some Message");

        let peak = log.peak();
        assert!(peak.is_some());
        assert_eq!(peak.unwrap(), "Some Messa");

        let take = log.take();
        assert!(take.is_some());
        assert_eq!(take.unwrap(), "Some Messa");

        let take = log.take();
        assert!(take.is_none());

        log.write("Message 1");
        log.write("Message 2");
        log.write("Message 3");
        log.write("Message 4");

        let peak = log.peak_n(3);
        assert!(peak.is_some());
        assert_eq!(peak.unwrap().len(), 3);

        let peak = log.peak_n(6);
        assert!(peak.is_some());
        assert_eq!(peak.unwrap().len(), 4);

        let take = log.take_n(3);
        assert!(take.is_some());
        assert_eq!(take.unwrap().len(), 3);

        let take = log.take_n(3);
        assert!(take.is_some());
        assert_eq!(take.unwrap().len(), 1);

        let take = log.take_n(3);
        assert!(take.is_none());

        let peak = log.peak_n(2);
        assert!(peak.is_none());
    }
}
