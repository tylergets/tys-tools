{
  lib,
  pkgs,
}:
pkgs.stdenv.mkDerivation {
  pname = "zonos3-native";
  version = "2026-11-15";
  src = pkgs.fetchzip {
    url = "https://github.com/Zyphra/ZONOS3-Mini/archive/96d9e0ec1e39083ed336ac78f24e3dbc93721c0f.tar.gz";
    hash = "sha256-gWTv+9mJ41mq5+LE30fw7FHiqsgxnMJaL6KlyFsE7YM=";
  };
  nativeBuildInputs = with pkgs; [cmake ninja git cargo rustc python3 makeWrapper];
  # The portable CPU backend is always available. Vulkan covers AMD/Intel/NVIDIA.
  buildInputs = with pkgs; [vulkan-headers vulkan-loader spirv-headers];
  postPatch = let
    ggml = pkgs.fetchzip {
      url = "https://github.com/ggml-org/ggml/archive/a05c9922613c6a2230dbb1d0de679519ecafb4f0.tar.gz";
      hash = "sha256-a/NtFGX7jYITnj6EWE6+H2eyhsKutn0k4IiStihOagA=";
    };
    textnorm = pkgs.fetchzip {
      url = "https://github.com/Zyphra/zyphra-textnorm/archive/170c571e75034c9a9f3bece9885c36d8dae77df2.tar.gz";
      hash = "sha256-TqS6tXvSsk37kMWlwINioFjwPiisYMs/UA4sEalLx6Q=";
    };
  in ''
    cp -r ${ggml}/. servers/ggml/ggml/
    cp -r ${textnorm}/. third_party/zyphra-textnorm/
    chmod -R u+w servers/ggml/ggml third_party/zyphra-textnorm
    # The normalizer library has no dependencies. Exclude its Python bindings and
    # test-only dependencies so CMake's cargo invocation is entirely offline.
    python3 - <<'PY'
    import pathlib, re
    p = pathlib.Path("third_party/zyphra-textnorm/Cargo.toml")
    text = p.read_text().replace('members = ["python"]', 'members = []')
    text = re.sub(r'\[dev-dependencies\].*?(?=\[\[bench\]\])', "", text, flags=re.S)
    p.write_text(text)
    PY
    rm third_party/zyphra-textnorm/Cargo.lock
    cargo generate-lockfile --offline --manifest-path third_party/zyphra-textnorm/Cargo.toml
  '';
  preConfigure = ''
    export CARGO_HOME=$TMPDIR/cargo
    export CARGO_NET_OFFLINE=true
  '';
  cmakeDir = "../servers/ggml";
  cmakeFlags = [
    "-DGGML_NATIVE=OFF"
    "-DBUILD_SHARED_LIBS=OFF"
    "-DGGML_VULKAN=ON"
    "-DVulkan_GLSLC_EXECUTABLE=${lib.getBin pkgs.shaderc}/bin/glslc"
  ];
  buildPhase = ''
    runHook preBuild
    cmake --build . --parallel "$NIX_BUILD_CORES" --target zonos3-server spk-encoder-cli
    runHook postBuild
  '';
  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    install -m755 zonos3-server spk-encoder-cli $out/bin/
    wrapProgram $out/bin/spk-encoder-cli --prefix PATH : ${lib.makeBinPath [pkgs.ffmpeg]}
    runHook postInstall
  '';
  meta = {
    description = "Native Zonos3-Mini streaming synthesis and speaker encoder";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
  };
}
