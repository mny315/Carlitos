{
  description = "Carlitos — a local Slint audiobook player";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { nixpkgs, ... }: let
    systems = [ "x86_64-linux" "aarch64-linux" ];
    forAll = nixpkgs.lib.genAttrs systems;
    environment = system: let
      pkgs = import nixpkgs { inherit system; };
      audio = import ./nix/audio.nix { inherit pkgs; };
      plugins = audio.packages;
      graphics = with pkgs; [ wayland libxkbcommon libGL fontconfig freetype libx11 libxcursor libxi libxrandr libxcb ];
    in { inherit pkgs plugins graphics audio; };
  in {
    packages = forAll (system: let
      inherit (environment system) pkgs plugins graphics audio;
      gstPath = audio.pluginPath;
      libraryPath = pkgs.lib.makeLibraryPath graphics;
    in rec {
      audio-runtime = pkgs.symlinkJoin {
        name = "carlitos-audio-runtime";
        paths = [ audio.core.out ] ++ plugins;
      };
      default = pkgs.rustPlatform.buildRustPackage {
        pname = "carlitos";
        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
        src = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./Cargo.toml ./Cargo.lock ./build.rs ./.cargo ./src ./ui ./data ./tests ./vendor ./LICENSE
          ];
        };
        cargoLock.lockFile = ./Cargo.lock;
        nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
        buildInputs = graphics ++ [ audio.core ] ++ plugins;
        GST_PLUGIN_SYSTEM_PATH_1_0 = gstPath;
        LD_LIBRARY_PATH = libraryPath;
        postInstall = ''
          install -Dm644 data/applications/io.github.mny315.Carlitos.desktop $out/share/applications/io.github.mny315.Carlitos.desktop
          install -Dm644 data/icons/io.github.mny315.Carlitos.svg $out/share/icons/hicolor/scalable/apps/io.github.mny315.Carlitos.svg
          install -Dm644 data/icons/io.github.mny315.Carlitos-symbolic.svg $out/share/icons/hicolor/symbolic/apps/io.github.mny315.Carlitos-symbolic.svg
          install -Dm644 LICENSE $out/share/licenses/carlitos/LICENSE
          install -Dm644 data/licenses/SLINT-LICENSE.md $out/share/licenses/carlitos/SLINT-LICENSE.md
          install -Dm644 data/licenses/THIRD-PARTY.md $out/share/licenses/carlitos/THIRD-PARTY.md
          mv $out/bin/carlitos $out/bin/Carlitos
          wrapProgram $out/bin/Carlitos \
            --set GST_PLUGIN_SYSTEM_PATH_1_0 "${gstPath}" \
            --set GST_PLUGIN_PATH_1_0 "" \
            --prefix LD_LIBRARY_PATH : "${libraryPath}"
        '';
        meta = with pkgs.lib; {
          description = "A calm, local audiobook player";
          platforms = platforms.linux;
          mainProgram = "Carlitos";
          license = licenses.mit;
        };
      };
      portable = let
        version = default.version;
        spec = pkgs.writeText "carlitos-portable.json" (builtins.toJSON {
          inherit version;
          recipes = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml ./Cargo.lock ./build.rs ./.cargo ./flake.nix ./flake.lock
              ./LICENSE ./build.sh
              ./src ./tests ./ui ./data ./scripts ./nix ./vendor
            ];
          };
          arch = pkgs.stdenv.hostPlatform.parsed.cpu.name;
          package = default;
          gstreamerVersion = audio.core.version;
          scanner = "${audio.core.out}/libexec/gstreamer-1.0/gst-plugin-scanner";
          pluginDirs = map (p: "${p}/lib/gstreamer-1.0") ([ audio.core.out ] ++ plugins);
          # Loaded with dlopen, so they are not necessarily in DT_NEEDED.
          libraries = [
            "${pkgs.wayland}/lib/libwayland-client.so.0"
            "${pkgs.wayland}/lib/libwayland-cursor.so.0"
            "${pkgs.wayland}/lib/libwayland-egl.so.1"
            "${pkgs.libxkbcommon}/lib/libxkbcommon.so.0"
            "${pkgs.libx11}/lib/libX11.so.6"
            "${pkgs.libxcursor}/lib/libXcursor.so.1"
            "${pkgs.libxi}/lib/libXi.so.6"
            "${pkgs.libxrandr}/lib/libXrandr.so.2"
            "${pkgs.libxcb}/lib/libxcb.so.1"
            "${pkgs.libGL}/lib/libGL.so.1"
            "${pkgs.libGL}/lib/libEGL.so.1"
            "${pkgs.libGL}/lib/libGLESv2.so.2"
          ];
          data = [
            { source = "${default}/share"; target = "share"; }
            { source = "${pkgs.xkeyboard_config}/share/X11/xkb"; target = "share/X11/xkb"; }
            { source = "${pkgs.libx11}/share/X11/locale"; target = "share/X11/locale"; }
            { source = "${pkgs.dejavu_fonts}/share/fonts/truetype/DejaVuSans.ttf"; target = "share/fonts/DejaVuSans.ttf"; }
            { source = "${pkgs.alsa-lib}/share/alsa"; target = "share/alsa"; }
            { source = ./flake.lock; target = "flake.lock"; }
            { source = ./Cargo.lock; target = "Cargo.lock"; }
          ] ++ map (p: { source = "${p}/share/licenses"; target = "share/licenses"; }) plugins;
        });
      in pkgs.runCommand "carlitos-${version}-portable" {
        nativeBuildInputs = [ pkgs.python3 pkgs.patchelf ];
      } ''
        python3 ${./scripts/portable.py} ${spec} "$out"
      '';
    });
    devShells = forAll (system: let inherit (environment system) pkgs plugins graphics audio; in {
      android = import ./nix/android.nix { inherit nixpkgs system; };
      windows = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [
          rustup cargo-xwin python3
          wineWow64Packages.stable xvfb-run
          llvmPackages.clang llvmPackages.lld llvmPackages.llvm
          # cc-rs needs the MSVC archive interface, which LLVM's Linux
          # package does not always install under the llvm-lib name.
          (writeShellScriptBin "llvm-lib" ''exec ${llvmPackages.lld}/bin/lld-link /lib "$@"'')
        ];
        CARLITOS_RUST_VERSION = pkgs.rustc.version;
        CARLITOS_INNO_VERSION = "6.7.3";
        CARLITOS_INNO_SETUP = pkgs.fetchurl {
          url = "https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe";
          hash = "sha256-nHPDuuftSNRBEqD0jmZ0LAAJC9tb73HZ08BWxm6XtzI=";
        };
      };
      default = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [ rustc cargo rustfmt clippy pkg-config ffmpeg grim wtype python3 dbus patchelf bubblewrap ];
        CARLITOS_TEST_BUSYBOX = "${pkgs.pkgsStatic.busybox}/bin/busybox";
        buildInputs = graphics ++ [ audio.core ] ++ plugins;
        GST_PLUGIN_SYSTEM_PATH_1_0 = audio.pluginPath;
        GST_PLUGIN_PATH_1_0 = "";
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath graphics;
      };
    });
  };
}
