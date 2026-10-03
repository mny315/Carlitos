{ nixpkgs, system }:
let
  versions = builtins.fromJSON (builtins.readFile ./android-versions.json);
  pkgs = import nixpkgs {
    inherit system;
    config = { allowUnfree = true; android_sdk.accept_license = true; };
  };
  android = pkgs.androidenv.composeAndroidPackages {
    platformVersions = [ (toString versions.compileSdk) ];
    buildToolsVersions = [ versions.buildTools ];
    includeNDK = true;
    ndkVersions = [ versions.ndk ];
    includeEmulator = false;
    includeSystemImages = false;
  };
in pkgs.mkShell {
  nativeBuildInputs = with pkgs; [ rustup jdk17 python3 curl unzip pkg-config clang ];
  ANDROID_HOME = "${android.androidsdk}/libexec/android-sdk";
  ANDROID_NDK_ROOT = "${android.androidsdk}/libexec/android-sdk/ndk/${versions.ndk}";
  JAVA_HOME = "${pkgs.jdk17}";
  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
  # rust-skia has no ARM32 prebuilt binaries. Its source build needs native
  # Nix executables instead of the downloaded generic Linux GN/Ninja tools.
  SKIA_GN_COMMAND = "${pkgs.gn}/bin/gn";
  SKIA_NINJA_COMMAND = "${pkgs.ninja}/bin/ninja";
  # AGP's downloaded Linux aapt2 is not patched for NixOS.
  CARLITOS_AAPT2 = "${android.androidsdk}/libexec/android-sdk/build-tools/${versions.buildTools}/aapt2";
}
