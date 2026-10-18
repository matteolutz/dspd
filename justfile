set windows-shell := ["powershell.exe", "-c"]

[group('project-agnostic')]
default:
    @just --list --justfile {{justfile()}}

[group("development")]
[arg("profile", pattern="release|debug")]
build package="" profile="debug":
    cargo build {{ if profile == "release" { "--release" } else { "" } }} {{ if package == "" { "" } else { f'-p {{package}}' } }}

[group("development")]
[confirm("Are you sure you want to clean the cargo target directory? (y/N)")]
clean:
    cargo clean

[group("development")]
debian debos="debos":
    @just --justfile {{justfile()}} build dspd release
    cp {{justfile_directory()}}/target/release/dspd {{justfile_directory()}}/debian/overlay/usr/bin/dspd
    sudo {{debos}} {{justfile_directory()}}/debian/debimage.yaml

[group("testing")]
test package="":
    cargo test {{ if package == "" { "" } else { f'-p {{package}}' } }} -- --nocapture
