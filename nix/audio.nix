{ pkgs }:
let
  inherit (pkgs) lib;
  gst = pkgs.gst_all_1;
  core = (gst.gstreamer.override {
    withIntrospection = false;
    withLibunwind = false;
    withRust = false;
    enableDocumentation = false;
  }).overrideAttrs (old: {
    # A documentation generator otherwise retains the entire Python runtime.
    postInstall = old.postInstall + ''
      rm -f "$out/libexec/gstreamer-1.0/gst-plugins-doc-cache-generator"
    '';
  });
  # Use the upstream sources pinned by flake.lock. Only explicitly enabled
  # features are built; adding a dependency cannot silently enable a codec.
  plugins = name: inputs: features: extraFlags:
    pkgs.stdenv.mkDerivation {
      pname = "carlitos-gst-${name}";
      inherit (gst."gst-plugins-${name}") version src;
      outputs = [ "out" "dev" ];
      strictDeps = true;
      nativeBuildInputs = with pkgs; [ meson ninja pkg-config python3 gettext glib orc ];
      buildInputs = [ core pkgs.orc ] ++ inputs;
      mesonFlags = [
        "-Dauto_features=disabled"
        "-Ddoc=disabled"
        "-Dtests=disabled"
        "-Dexamples=disabled"
        "-Dglib_debug=disabled"
        "-Dorc=enabled"
        "-Dorc-compiler=enabled"
      ] ++ map (feature: "-D${feature}=enabled") features ++ extraFlags;
      postPatch = ''
        patchShebangs scripts
      '';
      postInstall = ''
        install -Dm644 "$NIX_BUILD_TOP/$sourceRoot/COPYING" \
          "$out/share/licenses/gst-plugins-${name}/COPYING"
      '';
      meta.license = lib.licenses.lgpl2Plus;
    };
  base = plugins "base" [ pkgs.alsa-lib ] [
    "alsa" "audioconvert" "audioresample" "playback" "typefind" "volume"
  ] [ ];
  good = plugins "good" [ base pkgs.flac pkgs.libmpg123 pkgs.libpulseaudio pkgs.libjpeg pkgs.libpng ] [
    "audiofx" "audioparsers" "autodetect" "flac" "id3demux" "isomp4"
    "mpg123" "pulse" "wavparse" "jpeg" "png"
  ] [ ];
  # The existing FDK plugin covers AAC-LC and HE-AAC without shipping FFmpeg,
  # video codecs or the rest of gst-plugins-bad. Its library retains its license.
  aac = (plugins "bad" [ base pkgs.fdk_aac ] [ "fdkaac" ] [ ]).overrideAttrs (old: {
    postInstall = old.postInstall + ''
      mkdir -p "$out/share/licenses/fdk-aac"
      tar -xOf ${pkgs.fdk_aac.src} fdk-aac-${pkgs.fdk_aac.version}/NOTICE \
        > "$out/share/licenses/fdk-aac/NOTICE"
    '';
  });
  packages = [ base good aac ];
in {
  inherit base good aac packages core;
  pluginPath = lib.makeSearchPath "lib/gstreamer-1.0" ([ core.out ] ++ packages);
}
