# TCP-Share
### Access structs over TCP with 2 lines of code

### Features
- default = ["async-tcp"]
- async-tcp

### How to register?
```rs
#[register_impl]
impl MyStruct {
    pub async fn function1(self, input: u16) -> String { ... }
    pub async fn function2(self, input: u32) -> String { ... }
}

#[derive(TCPShare)]
struct MyStruct {}
```

### How to use(Server)?
```rs
MyStruct::default().start(8082, "struct-version").await;
```

### How to use(Client)?
```rs
let data = MyStruct::read(8082, "struct-version").function1(0);
```

### Info
- struct-version needs to be the same on both sides
- the `#[register_impl]` needs to be above the `#[derive(TCPShare)]`
- `{MyStruct}Reader` & `{MyStruct}Writer` structs are generated and therefor shouldnt be used
- `{MyStruct}` `start(self, port, identifier)` & `read(port, identifier)` are generated and therefor shouldnt be used

### Issues
- the `#[register_impl]` needs to be above the `#[derive(TCPShare)]`
- across files?
