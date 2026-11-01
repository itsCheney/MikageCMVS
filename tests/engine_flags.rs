use cmvs_rs::{
    engine_flags::EngineFlags,
    machine::{Machine, Step},
    ps3::Script,
};

fn reserve(code: &mut Vec<u8>, bytes: u16, raw: u32) {
    code.extend(0x440u16.to_le_bytes());
    code.extend(bytes.to_le_bytes());
    code.extend(raw.to_le_bytes());
}
/// Arguments are listed from SP-4 outward.
fn call(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for &arg in args.iter().rev() {
        reserve(code, 4, arg);
    }
    code.extend(op.to_le_bytes());
}
fn script(code: &[u8]) -> Script {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(code);
    let mut raw = b"PS2A".to_vec();
    for n in [
        48,
        0,
        0,
        1,
        code.len() as u32,
        0,
        0,
        0,
        0,
        payload.len() as u32,
        0,
    ] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}

#[test]
fn background_sound_defaults_on_and_answers_queries() {
    let mut code = vec![];
    call(&mut code, 0x22ed, &[0, 7]);
    call(&mut code, 0x22ed, &[1, 0]);
    call(&mut code, 0x22ed, &[0, 0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut flags = EngineFlags::default();
    for expected in [1, 1, 0] {
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 1).unwrap();
        assert!(flags.try_handle(&mut vm).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(expected));
    }
    assert!(!flags.background_sound);
}
