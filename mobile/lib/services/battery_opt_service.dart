import 'package:disable_battery_optimization/disable_battery_optimization.dart';
import 'package:flutter/services.dart';

/// Wrapper around the `disable_battery_optimization` plugin so the rest of
/// the app never has to deal with the platform channel directly.
///
/// Background:
/// On aggressive OEMs (Xiaomi MIUI, Huawei EMUI, OnePlus OxygenOS, Samsung
/// One UI ≥ 9) the system kills any background process within ~5 minutes
/// of the app being put into the background unless the user explicitly
/// granted "no battery optimisation" / "auto-start" / "protected app".
/// PhantomChat's relay-keeper and message-arrival path break under those
/// conditions. The opt-out flow is therefore an essential UX feature, not
/// just nice-to-have.
///
/// All methods are static + null-safe; an unsupported platform (e.g. iOS,
/// macOS, Linux desktop) yields the conservative answer `disabled = false`
/// and a no-op on the request side.
class BatteryOptService {
  /// Returns `true` if the user has already granted PhantomChat the "ignore
  /// battery optimisations" permission, `false` if optimisations are still
  /// active or if the platform does not support the query (in which case
  /// the UI should hide the section entirely).
  static Future<bool> isOptimizationDisabled() async {
    try {
      final raw = await DisableBatteryOptimization.isBatteryOptimizationDisabled;
      return raw ?? false;
    } on PlatformException {
      return false;
    } on MissingPluginException {
      return false;
    }
  }

  /// Pops the system "ignore battery optimisations" dialog. The plugin
  /// resolves to `true` once the user accepted, `false` on dismissal.
  static Future<bool> requestDisableOptimization() async {
    try {
      final raw = await DisableBatteryOptimization.showDisableBatteryOptimizationSettings();
      return raw ?? false;
    } on PlatformException {
      return false;
    } on MissingPluginException {
      return false;
    }
  }

  /// True when this build runs on a platform where the plugin can do
  /// anything meaningful (Android only as of plugin v1.1.x). Lets the UI
  /// hide the section completely on iOS/desktop.
  static Future<bool> platformSupported() async {
    try {
      // Probing the read-side is the cheapest way to detect "no plugin".
      await DisableBatteryOptimization.isBatteryOptimizationDisabled;
      return true;
    } on MissingPluginException {
      return false;
    } on PlatformException {
      return true; // plugin present but query failed — still supported.
    }
  }
}
