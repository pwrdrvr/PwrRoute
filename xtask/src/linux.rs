//! `cargo xtask package-linux`: a tar.gz and a .deb for the host
//! architecture, with a SHA-256 manifest, in `target/package`.

use std::{fs, path::Path, process::Command};

use anyhow::{Result, bail, ensure};

use crate::{package_version, root, run};

const SUMMARY: &str = "Every link to the right browser";
const DESCRIPTION: &str = "PwrRoute routes every link to the right browser and profile by rules.";

pub(crate) fn package() -> Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "package-linux runs on Linux only"
    );
    let root = root();
    let version = package_version()?;
    let (arch, deb_arch) = match std::env::consts::ARCH {
        "x86_64" => ("x64", "amd64"),
        "aarch64" => ("arm64", "arm64"),
        other => bail!("no Linux package for {other}"),
    };
    run(Command::new("cargo")
        .args(["build", "--release", "--locked", "-p", "pwrroute-app"])
        .current_dir(&root))?;
    let binary = root.join("target/release/pwrroute");
    let out = root.join("target/package");
    if out.exists() {
        fs::remove_dir_all(&out)?;
    }
    fs::create_dir_all(&out)?;

    // tar.gz: the binary, the desktop entry and the notices.
    let stem = format!("PwrRoute-{version}-linux-{arch}");
    let dir = out.join(&stem);
    fs::create_dir_all(&dir)?;
    fs::copy(&binary, dir.join("pwrroute"))?;
    fs::copy(
        root.join("packaging/linux/pwrroute.desktop"),
        dir.join("pwrroute.desktop"),
    )?;
    copy_notices(&root, &dir)?;
    run(Command::new("tar")
        .arg("-C")
        .arg(&out)
        .args(["-czf", &format!("{stem}.tar.gz"), &stem])
        .current_dir(&out))?;

    // .deb, built with dpkg-deb from a staged filesystem tree.
    let staging = out.join("deb");
    let bin = staging.join("usr/bin");
    let apps = staging.join("usr/share/applications");
    let docs = staging.join("usr/share/doc/pwrroute");
    for d in [&bin, &apps, &docs, &staging.join("DEBIAN")] {
        fs::create_dir_all(d)?;
    }
    fs::copy(&binary, bin.join("pwrroute"))?;
    fs::copy(
        root.join("packaging/linux/pwrroute.desktop"),
        apps.join("pwrroute.desktop"),
    )?;
    copy_notices(&root, &docs)?;
    // `~` sorts before the release, so 0.0.1~alpha.0 upgrades to 0.0.1; a
    // `-` would make "alpha.0" a Debian revision instead.
    let deb_version = version.replacen('-', "~", 1);
    fs::write(
        staging.join("DEBIAN/control"),
        format!(
            "Package: pwrroute\n\
             Version: {deb_version}\n\
             Architecture: {deb_arch}\n\
             Maintainer: PwrDrvr LLC <harold@pwrdrvr.com>\n\
             Section: net\n\
             Priority: optional\n\
             Homepage: https://github.com/pwrdrvr/PwrRoute\n\
             Description: {SUMMARY}\n {DESCRIPTION}\n"
        ),
    )?;
    let deb = out.join(format!("{stem}.deb"));
    run(Command::new("dpkg-deb")
        .args(["--build", "--root-owner-group"])
        .arg(&staging)
        .arg(&deb))?;
    fs::remove_dir_all(&staging)?;
    fs::remove_dir_all(&dir)?;

    run(Command::new("sh")
        .arg("-c")
        .arg(format!(
            "sha256sum -- *.deb *.tar.gz > PwrRoute-linux-{arch}-SHA256SUMS"
        ))
        .current_dir(&out))?;
    println!("{}", out.display());
    Ok(())
}

fn copy_notices(root: &Path, to: &Path) -> Result<()> {
    for file in ["LICENSE", "THIRD_PARTY_LICENSES", "README.md"] {
        fs::copy(root.join(file), to.join(file))?;
    }
    Ok(())
}
