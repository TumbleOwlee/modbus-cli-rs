#[cfg(test)]
mod tests {
    use crate::memory::{Memory, Range};
    use crate::register::{Definition, Handler, Type};
    use std::collections::HashMap;
    use std::sync::{Mutex, Arc};

    #[test]
    fn overlap() {
        let mut memory: Memory<10, _> = Memory::new();
        memory.init(&[Range::new(0u16, 40u16)]);
        let values: Vec<u16> = (17..34).map(|x| x as u16).collect();
        let _ = memory.write(Range::new(17u16, 34u16), &values);
        let vals = memory.read(&Range::new(16u16, 35u16)).unwrap();

        assert_eq!(
            vals.into_iter().copied().collect::<Vec<_>>(),
            vec![0, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 0]
        )
    }

    #[test]
    fn no_overlap() {
        let mut memory: Memory<1024, _> = Memory::new();
        memory.init(&[Range::new(0u16, 40u16)]);
        let values: Vec<u16> = (17..34).map(|x| x as u16).collect();
        let _ = memory.write(Range::new(17u16, 34u16), &values);
        let vals = memory.read(&Range::new(16u16, 35u16)).unwrap();

        assert_eq!(
            vals.into_iter().copied().collect::<Vec<_>>(),
            vec![0, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 0]
        )
    }

    #[test]
    fn register() {
        let mut memory: Memory<1024, u16> = Memory::new();
        memory.init(&[Range::new(0u16, 4096u16)]);
        let memory = Arc::new(Mutex::new(memory));
        let mut definitions: HashMap<String, Definition> = HashMap::new();
        definitions.insert(
            "Name".to_owned(),
            Definition::new(0, 2, Type::PackedString, 0x04u8),
        );
        let mut register = Handler::<1024>::new(&definitions, memory);
        assert!(register.update().is_ok());
    }
}
