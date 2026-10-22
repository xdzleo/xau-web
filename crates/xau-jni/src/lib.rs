//! The bridge between Kotlin and the Rust core.
//!
//! Every operation crosses as `call(method, argsJson) -> resultJson`, and
//! the commands themselves live in `xau-api` (shared with the web host).
//! This crate is only the JNI fence: it turns a JVM string into a method
//! and arguments, runs [`xau_api::dispatch`], and hands the answer back.
//!
//! # Errors cross as data
//!
//! A Rust panic unwinding into the JVM is undefined behaviour, so the
//! entry point catches unwinds and every failure comes back as
//! `{"err": "..."}`. Kotlin turns that into an exception on its side of
//! the fence, where an exception is safe.

use std::panic::{catch_unwind, AssertUnwindSafe};

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;
use serde_json::{json, Value};

use xau_core::error::Error;
use xau_mobile::accounts;
// --------------------------------------------------------------- entry

/// Initialise logging and tell the core where it may write.
///
/// `data_dir` must be the app's own `Context.getFilesDir()`. Rust cannot
/// work this out for itself on Android — the `directories` crate falls
/// back to `$HOME/.local/share`, which for an app process is `/` and is
/// not writable. Getting it from Java is the only correct answer.
///
/// `no_mangle` is what makes the symbol findable by name from the JVM;
/// the workspace warns on it because a duplicate export would be
/// undefined behaviour. Both symbols here are namespaced by the JNI
/// convention (`Java_<package>_<class>_<method>`), which no other
/// library in this process can collide with.
#[allow(unsafe_code)]
#[no_mangle]
pub extern "system" fn Java_com_xau_rewards_core_Native_init(
    mut env: JNIEnv,
    _class: JClass,
    data_dir: JString,
) {
    #[cfg(target_os = "android")]
    {
        // android_logger consumes `log` records. `tracing`'s "log"
        // feature re-emits every tracing event as one — but ONLY while
        // no tracing subscriber is installed. Installing a
        // tracing_subscriber registry here (which had no output layer
        // and so printed nothing itself) silently switched that bridge
        // off, and the app traced into a void.
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("xau"),
        );
    }

    // Read it unconditionally so the host contract is the same on every
    // target and a desktop test harness can point the store somewhere
    // temporary. An empty string means "you decide", which keeps the
    // desktop path working.
    if let Ok(dir) = env.get_string(&data_dir) {
        let dir: String = dir.into();
        if !dir.is_empty() {
            tracing::info!(%dir, "account store directory");
            accounts::set_data_dir(dir);
        }
    }
}

/// The single crossing point. See the module docs for why there is one.
#[allow(unsafe_code)] // JNI-namespaced export; see `Native_init` above.
#[no_mangle]
pub extern "system" fn Java_com_xau_rewards_core_Native_call(
    mut env: JNIEnv,
    _class: JClass,
    method: JString,
    args: JString,
) -> jstring {
    // A panic unwinding into the JVM is undefined behaviour, so nothing
    // below may escape. `AssertUnwindSafe` is honest here: the state a
    // panic could leave inconsistent is the session map, and a poisoned
    // lock is recovered explicitly everywhere it is taken.
    let out = catch_unwind(AssertUnwindSafe(|| {
        let method: String = env
            .get_string(&method)
            .map_err(|e| Error::other(e.to_string()))?
            .into();
        let args: String = env
            .get_string(&args)
            .map_err(|e| Error::other(e.to_string()))?
            .into();
        let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
        xau_api::runtime()?.block_on(xau_api::dispatch(&method, &args))
    }));

    let body = match out {
        Ok(Ok(v)) => json!({ "ok": v }),
        Ok(Err(e)) => json!({ "err": e.to_string() }),
        Err(_) => json!({ "err": "falha interna inesperada" }),
    };

    // If even the response string cannot be built there is nothing left
    // to report with, so hand back null and let Kotlin raise.
    env.new_string(body.to_string())
        .map_or(std::ptr::null_mut(), JString::into_raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fence must hand an unknown method back as data. It reaches the
    /// shared core, so this also pins that the JNI crate is wired to it.
    #[test]
    fn an_unknown_method_comes_back_as_an_error() {
        let rt = xau_api::runtime().expect("runtime");
        let r = rt.block_on(xau_api::dispatch("does_not_exist", &Value::Null));
        assert!(r.is_err_and(|e| e.to_string().contains("desconhecido")));
    }
}