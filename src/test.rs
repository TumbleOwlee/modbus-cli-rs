#[cfg(test)]
mod tests {
    use crate::mem::data::DataType;
    use crate::mem::memory::{Memory, Range};
    use crate::mem::register::{AccessType, Definition, Handler};
    use crate::AppConfig;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[test]
    fn overlap() {
        let mut memory = Memory::new();
        memory.init(0, &[Range::new(0u16, 40u16)]);
        let values: Vec<u16> = (17..34).map(|x| x as u16).collect();
        let _ = memory.write(0, Range::new(17u16, 34u16), &values);
        let vals = memory.read(0, &Range::new(16u16, 35u16)).unwrap();

        assert_eq!(
            vals.into_iter().copied().collect::<Vec<_>>(),
            vec![0, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 0]
        )
    }

    #[test]
    fn no_overlap() {
        let mut memory = Memory::new();
        memory.init(0, &[Range::new(0u16, 40u16)]);
        let values: Vec<u16> = (17..34).map(|x| x as u16).collect();
        let _ = memory.write(0, Range::new(17u16, 34u16), &values);
        let vals = memory.read(0, &Range::new(16u16, 35u16)).unwrap();

        assert_eq!(
            vals.into_iter().copied().collect::<Vec<_>>(),
            vec![0, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 0]
        )
    }

    #[test]
    fn register() {
        let mut memory = Memory::new();
        memory.init(0, &[Range::new(0u16, 4096u16)]);
        let memory = Arc::new(Mutex::new(memory));
        let mut definitions: HashMap<String, Definition> = HashMap::new();
        definitions.insert(
            "Name".to_owned(),
            Definition::new(
                None,
                0,
                2,
                DataType::default(),
                0x04u8,
                AccessType::ReadOnly,
                None,
            ),
        );
        let config = Arc::new(Mutex::new(AppConfig::default()));
        let mut register = Handler::new(config, memory);
        register
            .set_values(0, 1234, &[0x1234, 0x2345])
            .expect("Set values failed");
    }
}
