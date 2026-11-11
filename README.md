<!-- markdownlint-disable MD033 -->
<!-- markdownlint-disable MD041 -->
<p align="left">
<img width="350" alt="Lumina logo" src="https://raw.githubusercontent.com/richbl/rust-lumina/refs/heads/main/.github/assets/ui/lumina_logo_title.png">
</p>
<!-- markdownlint-enable MD041 -->

<a href="https://github.com/richbl/rust-lumina/releases"><img align="middle" alt="Link to Releases" src="https://badgen.net/github/tag/richbl/rust-lumina?icon=github&label=release"></a>
<a href="https://github.com/richbl/rust-lumina/pulls?q=is%3Apr+state%3Aclosed"><img align="middle" alt="Link to PRs" src="https://badgen.net/github/last-commit/richbl/rust-lumina?color=blue&icon=github"></a>
<a href="https://app.codacy.com/gh/richbl/rust-lumina/dashboard?utm_source=gh&utm_medium=referral&utm_content=&utm_campaign=Badge_grade"><img align="middle" alt="Link to Codacy Report" src="https://app.codacy.com/project/badge/Grade/595889e53f25475da18dea64b5a60419"></a>
<a href="https://sonarcloud.io/summary/new_code?id=richbl_rust-lumina"><img align="middle" alt="Link to SonarCloud Report" src="https://sonarcloud.io/api/project_badges/measure?project=richbl_rust-lumina&metric=alert_status"></a>

<!-- <a href="https://codeberg.org/richbl/rust-lumina"><img align="middle" width="85" alt="Gitberg logo" src="https://raw.githubusercontent.com/richbl/rust-cli-sysinfo/refs/heads/main/.github/assets/gitberg_logo.png"></a> -->
<!-- markdownlint-enable MD033 -->

Automatically dims/brightens device displays based on location and time of day

## Overview

**Lumina** is a Rust-based application designed to automatically manage the screen brightness of device display(s) over the course of the day. It does this by determining your location (via a query to a public website API, or via manual input). Based on your location, the application then creates a "brightness curve" starting from sunrise, peaking in brightness around mid-day, and then slowly reducing brightness until sunset.

<!-- markdownlint-disable MD033 -->
<p align="center">
<img width="60%" alt="Screenshot showing app status" src="https://raw.githubusercontent.com/richbl/rust-lumina/refs/heads/main/.github/assets/ui/gui_status.png">
</p>

<p align="center">
<img width="60%" alt="Screenshot showing app settings" src="https://raw.githubusercontent.com/richbl/rust-lumina/refs/heads/main/.github/assets/ui/gui_settings.png">
</p>

<!-- markdownlint-enable MD033 -->

## Features

* Choice of **Lumina** running modes:

  * GUI Mode: a modern GTK4/Adwaita design
    * All configuration settings can be made via graphical controls
    * Automated saving of configuration details

  * CLI Mode: a simple command-line interface with minimal operational overhead
    * Application flags for commands and options to override configuration file settings

* Highly configurable TOML-based configuration files for:
  * Device display(s) brightness min/max values (`%`)
  * Location (latitude/longitude) (`+/- 00.0000000`)
  * Scheduling
    * Start time-of-day (`HH:MM`)
    * Stop time-of-day (`HH:MM`)
    * Brightness update interval (seconds)
    * Manual schedule override (`true/false`)

* Graceful handling of connection interrupts and system signals for a clean shutdown

* Integrated application installer/uninstaller for easy setup and self-removal of the application

## Rationale

Why build **Lumina**? To automate device brightness. I'm often in front of a computer over the course of the day, and in the Pacific Northwest, that usually means "dark when I start my day" and "dark when I end my day." So, **Lumina** makes my life just a bit easier so I don't have to mess with brightness controls on a daily basis.

## How Brightness Works

Brightness follows a smooth half-sine curve across a "daytime window" (sunrise to sunset unless overridden): `min` at the start, peak `max` at the midpoint, back to `min` at the stop, and `min` overnight.

<!-- markdownlint-disable MD033 -->
<p align="center">
<img width="85%" alt="Daytime solar window" src="https://raw.githubusercontent.com/richbl/rust-lumina/refs/heads/main/.github/assets/solar_window.png">
</p>

<!-- markdownlint-enable MD033 -->

## Configuration

Regardless whether you're running **Lumina** in CLI Mode or GUI Mode, the application creates a default configuration file in `~/.config/lumina/config.toml`:

    # Lumina Configuration (TOML)
    # v0.1.0

    [location]
    latitude = 47.9789803
    longitude = -122.2020794

    [brightness]
    min = 25
    max = 75

    [schedule]
    start_time = "07:08"
    stop_time = "16:39"
    interval_seconds = 5
    respect_manual_changes = true

Note that all sections and keys are optional, and command line flags override these options on a per-session basis.

## The Lumina GUI Application

In addition to running via the command line, `Lumina` can run as a stand-alone GUI application based on GTK4/Libadwaita using the Relm4 crate.

## Building the **Lumina** Application

If you plan to compile/build `Lumina` on your machine, you'll need the following packages:

* Debian/Ubuntu build dependencies (libadwaita >= 1.4 required):

      sudo apt install libgtk-4-dev libadwaita-1-dev pkg-config

* Fedora build dependencies:

      sudo dnf install gtk4-devel libadwaita-devel
