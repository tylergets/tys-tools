{
  lib,
  pkgs,
}:
pkgs.stdenv.mkDerivation {
  pname = "qwen-tts";
  version = "2026-11-15";
  src = pkgs.fetchzip {
    url = "https://github.com/ServeurpersoCom/qwentts.cpp/archive/f1a6924789883bd92eeb59176793a94af55730b3.tar.gz";
    hash = "sha256-7WxZPJ8SSNXqFqqCAjHt2RgfWOWrlBkdSCHzIjmxRe4=";
  };
  nativeBuildInputs = with pkgs; [cmake ninja shaderc];
  buildInputs = with pkgs; [vulkan-headers vulkan-loader];
  cmakeFlags = [
    "-DGGML_SOURCE_DIR=${pkgs.fetchzip {
      url = "https://github.com/ServeurpersoCom/ggml/archive/0d403add5cb6329935290f02462e1072045aacd2.tar.gz";
      hash = "sha256-E5/XQHrtRvNGvc27oM/3zASdTv3mOy0KkzTWQCRMoGk=";
    }}"
    "-DGGML_VULKAN=ON"
    "-DGGML_NATIVE=OFF"
    "-DBUILD_SHARED_LIBS=OFF"
    "-DGGML_AVX=ON"
    "-DGGML_AVX2=ON"
    "-DGGML_FMA=ON"
    "-DGGML_F16C=ON"
  ];
  ninjaFlags = ["qwen-tts" "qwen-codec"];
  installPhase = ''
    runHook preInstall
    install -Dm755 qwen-tts $out/bin/qwen-tts
    install -Dm755 qwen-codec $out/bin/qwen-codec
    runHook postInstall
  '';
  meta = {
    description = "Native Qwen3-TTS inference with Vulkan acceleration";
    homepage = "https://github.com/ServeurpersoCom/qwentts.cpp";
    license = lib.licenses.mit;
    platforms = ["x86_64-linux"];
  };
}
