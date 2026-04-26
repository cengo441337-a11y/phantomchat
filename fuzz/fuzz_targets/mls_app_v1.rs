// Fuzz the MLS-APP1 application-message decrypt path. Today the public
// entry point is `PhantomMlsGroup::decrypt`, which calls into openmls's
// `MlsMessageIn::tls_deserialize_exact` and then process_message().
//
// To exercise that we need a live group. We bootstrap one per fuzz run
// so the harness is fully self-contained.
#![no_main]
use libfuzzer_sys::fuzz_target;
use phantomchat_core::mls::PhantomMlsMember;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    // Bootstrap a one-member group; decrypt arbitrary bytes against it.
    let Ok(mut member) = PhantomMlsMember::new(b"fuzz-app-id".to_vec()) else {
        return;
    };
    let Ok(mut group) = member.create_group() else {
        return;
    };
    let _ = group.decrypt(data);
});
