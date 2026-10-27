//! What Task Manager asks the phone for. On Android this goes through the Java `SysInfo` class and
//! plain file reads; on a PC (the desktop preview) there is made-up data so the screens can be seen.

use crate::sample::{BootRow, parse_boot_line};
use eframe::egui::ColorImage;

/// ARGB pixels (as Android gives them) to an egui image.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn image_from_argb(px: &[i32], size: usize) -> ColorImage {
    let rgba: Vec<u8> = px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8]).collect();
    ColorImage::from_rgba_unmultiplied([size, size], &rgba)
}

/// Wall clock in milliseconds. Only the desktop preview's made-up readings need it; on a phone the
/// numbers come from Android, which stamps them itself.
#[cfg(not(target_os = "android"))]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A whole small file, or None when it does not exist or Android does not let this app read it.
pub fn read_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn parse_boots(lines: Vec<String>) -> Vec<BootRow> {
    lines.iter().filter_map(|l| parse_boot_line(l)).collect()
}

#[cfg(target_os = "android")]
pub use android::*;

#[cfg(not(target_os = "android"))]
pub use desktop::*;

#[cfg(not(target_os = "android"))]
mod desktop {
    use super::*;

    #[derive(Clone, Default)]
    pub struct Sys;

    fn wave(period: f64, lo: f64, hi: f64) -> f64 {
        let t = now_ms() as f64 / 1000.0;
        lo + (hi - lo) * (0.5 + 0.5 * (t * std::f64::consts::TAU / period).sin())
    }

    impl Sys {
        pub fn sample_text(&self) -> String {
            let t = now_ms() as f64 / 1000.0;
            let mut s = String::new();
            let mut kv = |k: &str, v: String| s.push_str(&format!("{k}\t{v}\n"));
            kv("mem_total", (8u64 << 30).to_string());
            kv("mem_avail", (((3.0 + wave(20.0, 0.0, 2.0)) * (1u64 << 30) as f64) as u64).to_string());
            kv("thermal", "1".into());
            kv("headroom", format!("{:.2}", wave(30.0, 0.5, 0.9)));
            kv("storage_total", (256u64 << 30).to_string());
            kv("storage_free", (90u64 << 30).to_string());
            kv("net_rx", ((t * 400_000.0 * (1.0 + wave(7.0, 0.0, 3.0))) as u64).to_string());
            kv("net_tx", ((t * 60_000.0) as u64).to_string());
            kv("uptime_ms", ((t * 1000.0) as i64 % 900_000_000).to_string());
            kv("bat_level", "73".into());
            kv("bat_scale", "100".into());
            kv("bat_status", "2".into());
            kv("bat_plugged", "2".into());
            kv("bat_health", "2".into());
            kv("bat_temp", format!("{:.0}", wave(40.0, 300.0, 380.0)));
            kv("bat_voltage", "4050".into());
            kv("bat_current_ua", "-1800000".into());
            kv("bat_cycles", "142".into());
            kv("bat_full_in_ms", "2400000".into());
            s
        }

        pub fn device_text(&self) -> String {
            "model\tDemo Phone\nandroid\t16 (API 36)\nkernel\t6.1.0-demo\nabi\tarm64-v8a\ncores\t8\nrefresh\t120.0\n".to_string()
        }


        pub fn boot_apps(&self) -> Vec<BootRow> {
            ["WhatsApp", "Messages", "Clock", "Maps", "Music"]
                .iter()
                .map(|n| BootRow { pkg: format!("demo.{}", n.to_lowercase()), label: (*n).to_string(), system: matches!(*n, "Messages" | "Clock") })
                .collect()
        }

        pub fn icon(&self, pkg: &str, px: usize) -> Option<ColorImage> {
            let seed = pkg.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
            let (r, g, b) = (80 + (seed % 150) as u8, 80 + ((seed >> 8) % 150) as u8, 80 + ((seed >> 16) % 150) as u8);
            let mut pixels = Vec::with_capacity(px * px);
            for y in 0..px {
                for x in 0..px {
                    let (dx, dy) = ((x as f32 - px as f32 / 2.0).abs(), (y as f32 - px as f32 / 2.0).abs());
                    let inside = dx.max(dy) < px as f32 / 2.0 - 2.0 && (dx * dx + dy * dy).sqrt() < px as f32 * 0.62;
                    pixels.push(if inside { eframe::egui::Color32::from_rgb(r, g, b) } else { eframe::egui::Color32::TRANSPARENT });
                }
            }
            Some(ColorImage::new([px, px], pixels))
        }
        pub fn app_info(&self, pkg: &str) {
            println!("would open the settings page of {pkg}");
        }
        pub fn battery_settings(&self) {
            println!("would open battery optimisation settings");
        }

        // detailed mode (nothing to pair with on a PC)
        pub fn nsd_start(&self) {}
        pub fn nsd_stop(&self) {}
        pub fn nsd_ports(&self) -> (u16, u16) {
            (0, 0)
        }
        pub fn show_pairing(&self, title: &str, text: &str, _input: bool) {
            println!("notification: {title} - {text}");
        }
        pub fn native_lib_dir(&self) -> String {
            "/nonexistent".to_string()
        }
        pub fn write_token(&self, token: &str) -> Option<String> {
            let dir = std::env::temp_dir().join("taskmgr-data");
            std::fs::create_dir_all(&dir).ok()?;
            let path = dir.join("token");
            std::fs::write(&path, token).ok()?;
            Some(path.to_string_lossy().to_string())
        }
        pub fn open_developer_options(&self) {
            println!("would open developer options");
        }
        pub fn can_notify(&self) -> bool {
            true
        }
        pub fn request_notify_permission(&self) {}

        // Shizuku (not on a PC)
        pub fn shizuku_state(&self) -> u32 {
            0
        }
        pub fn shizuku_request(&self) {}
        pub fn shizuku_open(&self) {
            println!("would open Shizuku");
        }
        pub fn shizuku_run(&self, _script: &str) -> String {
            "ERROR: Shizuku is not available on a PC".to_string()
        }
    }
}

#[cfg(target_os = "android")]
mod android {
    use super::*;
    use android_activity::AndroidApp;
    use jni::objects::{JIntArray, JObjectArray, JString, JValue};
    use heartnet_kit::android::{class, with_env};

    const HELPER: &str = "dev.heartnetapps.taskmanager.SysInfo";
    const ADV: &str = "dev.heartnetapps.taskmanager.AdvHelper";
    const NSD: &str = "dev.heartnetapps.taskmanager.NsdHelper";
    const SHIZUKU: &str = "dev.heartnetapps.taskmanager.ShizukuHelper";
    const RECEIVER: &str = "dev.heartnetapps.taskmanager.PairReceiver";

    /// `PairReceiver.nativeCode(String)`: the digits typed into the notification.
    extern "system" fn native_code(mut env: jni::JNIEnv, _class: jni::objects::JClass, code: JString) {
        if let Ok(s) = env.get_string(&code) {
            crate::adv::deliver_code(&String::from(s));
        }
    }

    #[derive(Clone)]
    pub struct Sys {
        app: AndroidApp,
    }

    impl Sys {
        pub fn new(app: AndroidApp) -> Self {
            let sys = Sys { app };
            sys.register_natives();
            sys
        }

        fn register_natives(&self) {
            with_env(&self.app, |env, act| {
                let c = class(env, act, RECEIVER)?;
                env.register_native_methods(&c, &[jni::NativeMethod { name: "nativeCode".into(), sig: "(Ljava/lang/String;)V".into(), fn_ptr: native_code as *mut std::ffi::c_void }])?;
                Ok(())
            });
        }

        fn text(&self, method: &str) -> String {
            with_env(&self.app, |env, act| {
                let c = class(env, act, HELPER)?;
                let s = env.call_static_method(&c, method, "(Landroid/app/Activity;)Ljava/lang/String;", &[JValue::Object(act)])?.l()?;
                let s: String = env.get_string(&JString::from(s))?.into();
                Ok(s)
            })
            .unwrap_or_default()
        }

        fn lines(&self, method: &str) -> Vec<String> {
            with_env(&self.app, |env, act| {
                let c = class(env, act, HELPER)?;
                let arr = env.call_static_method(&c, method, "(Landroid/app/Activity;)[Ljava/lang/String;", &[JValue::Object(act)])?.l()?;
                let arr = JObjectArray::from(arr);
                let n = env.get_array_length(&arr)?;
                let mut out = Vec::with_capacity(n as usize);
                for i in 0..n {
                    let s = JString::from(env.get_object_array_element(&arr, i)?);
                    out.push(env.get_string(&s)?.into());
                }
                Ok(out)
            })
            .unwrap_or_default()
        }

        fn pkg_void(&self, method: &str, pkg: &str) {
            with_env(&self.app, |env, act| {
                let c = class(env, act, HELPER)?;
                let p = env.new_string(pkg)?;
                env.call_static_method(&c, method, "(Landroid/app/Activity;Ljava/lang/String;)V", &[JValue::Object(act), JValue::Object(&p)])?;
                Ok(())
            });
        }

        pub fn sample_text(&self) -> String {
            self.text("sample")
        }

        pub fn device_text(&self) -> String {
            self.text("device")
        }


        pub fn boot_apps(&self) -> Vec<BootRow> {
            parse_boots(self.lines("bootApps"))
        }




        pub fn icon(&self, pkg: &str, px: usize) -> Option<ColorImage> {
            with_env(&self.app, |env, act| {
                let c = class(env, act, HELPER)?;
                let p = env.new_string(pkg)?;
                let arr = env
                    .call_static_method(&c, "icon", "(Landroid/app/Activity;Ljava/lang/String;I)[I", &[JValue::Object(act), JValue::Object(&p), JValue::Int(px as i32)])?
                    .l()?;
                let arr = JIntArray::from(arr);
                let mut buf = vec![0i32; px * px];
                env.get_int_array_region(&arr, 0, &mut buf)?;
                Ok(image_from_argb(&buf, px))
            })
        }


        pub fn app_info(&self, pkg: &str) {
            self.pkg_void("appInfo", pkg)
        }



        pub fn battery_settings(&self) {
            with_env(&self.app, |env, act| {
                let c = class(env, act, HELPER)?;
                env.call_static_method(&c, "batterySettings", "(Landroid/app/Activity;)V", &[JValue::Object(act)])?;
                Ok(())
            });
        }

        // ---------- detailed mode ----------

        fn adv_call(&self, method: &str) {
            with_env(&self.app, |env, act| {
                let c = class(env, act, ADV)?;
                env.call_static_method(&c, method, "(Landroid/app/Activity;)V", &[JValue::Object(act)])?;
                Ok(())
            });
        }

        pub fn nsd_start(&self) {
            with_env(&self.app, |env, act| {
                let c = class(env, act, NSD)?;
                env.call_static_method(&c, "start", "(Landroid/app/Activity;)V", &[JValue::Object(act)])?;
                Ok(())
            });
        }

        pub fn nsd_stop(&self) {
            with_env(&self.app, |env, act| {
                let c = class(env, act, NSD)?;
                env.call_static_method(&c, "stop", "()V", &[])?;
                Ok(())
            });
        }

        /// (connect port, pairing port); 0 where not found.
        pub fn nsd_ports(&self) -> (u16, u16) {
            let text = with_env(&self.app, |env, act| {
                let c = class(env, act, NSD)?;
                let s = env.call_static_method(&c, "state", "()Ljava/lang/String;", &[])?.l()?;
                let s: String = env.get_string(&JString::from(s))?.into();
                Ok(s)
            })
            .unwrap_or_default();
            let kv = crate::sample::parse_kv(&text);
            let port = |k: &str| crate::sample::get_i64(&kv, k).and_then(|p| u16::try_from(p).ok()).unwrap_or(0);
            (port("connect"), port("pair"))
        }

        pub fn show_pairing(&self, title: &str, text: &str, input: bool) {
            with_env(&self.app, |env, act| {
                let c = class(env, act, ADV)?;
                let (t, x) = (env.new_string(title)?, env.new_string(text)?);
                env.call_static_method(
                    &c,
                    "showPairing",
                    "(Landroid/app/Activity;Ljava/lang/String;Ljava/lang/String;Z)V",
                    &[JValue::Object(act), JValue::Object(&t), JValue::Object(&x), JValue::Bool(input as u8)],
                )?;
                Ok(())
            });
        }

        pub fn native_lib_dir(&self) -> String {
            with_env(&self.app, |env, act| {
                let c = class(env, act, ADV)?;
                let s = env.call_static_method(&c, "nativeLibDir", "(Landroid/app/Activity;)Ljava/lang/String;", &[JValue::Object(act)])?.l()?;
                let s: String = env.get_string(&JString::from(s))?.into();
                Ok(s)
            })
            .unwrap_or_default()
        }

        /// Save the helper's token where the phone's shell can read it; returns the path.
        pub fn write_token(&self, token: &str) -> Option<String> {
            let dir = with_env(&self.app, |env, act| {
                let c = class(env, act, ADV)?;
                let s = env.call_static_method(&c, "externalFilesDir", "(Landroid/app/Activity;)Ljava/lang/String;", &[JValue::Object(act)])?.l()?;
                let s: String = env.get_string(&JString::from(s))?.into();
                Ok(s)
            })?;
            let path = format!("{dir}/token");
            std::fs::write(&path, token).ok()?;
            Some(path)
        }

        pub fn open_developer_options(&self) {
            self.adv_call("openDeveloperOptions");
        }

        pub fn can_notify(&self) -> bool {
            with_env(&self.app, |env, act| {
                let c = class(env, act, ADV)?;
                env.call_static_method(&c, "canNotify", "(Landroid/app/Activity;)Z", &[JValue::Object(act)])?.z()
            })
            .unwrap_or(false)
        }

        pub fn request_notify_permission(&self) {
            self.adv_call("requestNotifyPermission");
        }

        // ---------- Shizuku ----------

        /// Bits: 1 installed, 2 running, 4 this app is allowed.
        pub fn shizuku_state(&self) -> u32 {
            with_env(&self.app, |env, act| {
                let c = class(env, act, SHIZUKU)?;
                Ok(env.call_static_method(&c, "state", "(Landroid/app/Activity;)I", &[JValue::Object(act)])?.i()? as u32)
            })
            .unwrap_or(0)
        }

        pub fn shizuku_request(&self) {
            with_env(&self.app, |env, act| {
                let c = class(env, act, SHIZUKU)?;
                env.call_static_method(&c, "requestPermission", "()V", &[])?;
                Ok(())
            });
        }

        pub fn shizuku_open(&self) {
            with_env(&self.app, |env, act| {
                let c = class(env, act, SHIZUKU)?;
                env.call_static_method(&c, "open", "(Landroid/app/Activity;)V", &[JValue::Object(act)])?;
                Ok(())
            });
        }

        /// Run a shell line through Shizuku; failures come back as text starting with "ERROR:".
        pub fn shizuku_run(&self, script: &str) -> String {
            with_env(&self.app, |env, act| {
                let c = class(env, act, SHIZUKU)?;
                let s = env.new_string(script)?;
                let r = env.call_static_method(&c, "run", "(Ljava/lang/String;J)Ljava/lang/String;", &[JValue::Object(&s), JValue::Long(30_000)])?.l()?;
                let r: String = env.get_string(&JString::from(r))?.into();
                Ok(r)
            })
            .unwrap_or_else(|| "ERROR: could not call Shizuku".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_pixels_become_an_image() {
        let px = [0xFF102030u32 as i32, 0x80FF0000u32 as i32, 0, 0xFFFFFFFFu32 as i32];
        let img = image_from_argb(&px, 2);
        assert_eq!(img.size, [2, 2]);
        assert_eq!((img.pixels[0].r(), img.pixels[0].g(), img.pixels[0].b(), img.pixels[0].a()), (0x10, 0x20, 0x30, 255));
        assert_eq!(img.pixels[1].a(), 0x80);
    }

    #[test]
    fn broken_lines_are_skipped_when_lists_are_read() {
        let boots = parse_boots(vec!["com.a	Alpha	1".into(), "".into()]);
        assert_eq!(boots.len(), 1);
        assert!(boots[0].system);
    }
}
