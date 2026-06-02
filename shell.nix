{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    cargo
    rustc
    rustfmt
    clippy
    rust-analyzer
  ];

  # 필요한 경우 환경 변수 설정
  shellHook = ''
    export RUST_SRC_PATH=${pkgs.rustPlatform.rustLibSrc}
    echo "🦀 Welcome to the Rust development environment! 🦀"
    rustc --version
  '';
}
