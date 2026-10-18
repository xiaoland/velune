use jni::{
    EnvUnowned, errors::ThrowRuntimeExAndDefault, jni_mangle, jni_str, objects::JObject,
    refs::Global,
};
use std::sync::OnceLock;

static APPLICATION_CONTEXT: OnceLock<Global<JObject<'static>>> = OnceLock::new();

#[jni_mangle("app.velune.android.NativePlatform")]
pub fn initialize<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _receiver: JObject<'caller>,
    context: JObject<'caller>,
) {
    unowned_env
        .with_env(|env| {
            env.find_class(jni_str!("org/rustls/platformverifier/CertificateVerifier"))?;
            let global = env.new_global_ref(&context)?;
            let vm = env.get_java_vm()?;
            rustls_platform_verifier::android::init_with_env(env, context)?;
            APPLICATION_CONTEXT.get_or_init(|| {
                // iroh retains raw JNI pointers; this global reference lives until process exit.
                unsafe {
                    velune_remote::install_android_jni_context(
                        vm.get_raw().cast(),
                        global.as_raw().cast(),
                    );
                }
                global
            });
            Ok::<_, jni::errors::Error>(())
        })
        .resolve::<ThrowRuntimeExAndDefault>();
}
