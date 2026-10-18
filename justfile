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

[group("debian")]
debian debos="debos":
    @just --justfile {{justfile()}} build dspd release
    cp {{justfile_directory()}}/target/release/dspd {{justfile_directory()}}/debian/overlay/usr/local/bin/dspd
    sudo {{debos}} {{justfile_directory()}}/debian/debimage.yaml

[group("debian")]
[confirm("Are you sure you want to flash the debian image to \"" + device + "\"? (y/N)")]
flash device img="":
	sudo bmaptool copy {{ if img =="" { f"{{justfile_directory()}}/debian-dspd-trixie.img.gz" } else { img } }} {{device}}

[group("testing")]
test package="":
    cargo test {{ if package == "" { "" } else { f'-p {{package}}' } }} -- --nocapture
