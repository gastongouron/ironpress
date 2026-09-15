package io.github.gastongouron.ironpress;

/** Version information for the packaged Ironpress native library. */
public final class IronpressInfo {
  /** C ABI generation required by this Java package. */
  public static final int REQUIRED_ABI_VERSION = 1;

  /** Ironpress version carried by this Maven artifact. */
  public static final String PACKAGE_VERSION = "1.7.0";

  private IronpressInfo() {}

  /** Return the C ABI generation implemented by the loaded native library. */
  public static int abiVersion() {
    return NativeLibraryLoader.api().ironpress_abi_version();
  }

  /** Return the Ironpress version implemented by the loaded native library. */
  public static String version() {
    return NativeLibraryLoader.nativeVersion();
  }
}
