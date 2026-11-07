use std::{
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

use tokio::{io, time::Instant};

#[derive(Default)]
pub struct JackDaemonRealtimeConfig {
    /// Whether to enable JACK realtime mode. If `false`, the JACK client will run in non-realtime mode.
    pub enabled: bool,
    /// Linux realtime priority to use for the JACK client. If `None`, the default priority will be used.
    /// Range: 1-99, where 1 is the lowest priority and 99 is the highest.
    pub priority: Option<u8>,
}

impl JackDaemonRealtimeConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(priority) = self.priority {
            if priority < 1 || priority > 99 {
                anyhow::bail!("Realtime priority must be between 1 and 99");
            }
        }

        Ok(())
    }
}

pub struct JackDaemonConfig {
    /// The sample rate to use for the JACK server. This will be passed with `-r` to the JACK daemon.
    pub sample_rate: u32,

    /// The number of frames per period. This is the number of frames that will be processed in each callback.
    /// This will be passed with `-p` to the JACK daemon.
    pub period_size: u32,

    /// The number of periods per buffer.
    /// This will be passed with `-n` to the JACK daemon.
    pub periods: u32,

    /// The backend to use for the JACK server. This will be passed with `-d` to the JACK daemon.
    /// Defaults to `alsa`
    pub backend: String,

    /// The capture device to use for the JACK server. This will be passed with `-C` to the JACK daemon.
    /// Defaults to `hw:0`
    pub capture_device: Option<String>,

    /// The playback device to use for the JACK server. This will be passed with `-P` to the JACK daemon.
    /// Defaults to `hw:0`
    pub playback_device: Option<String>,

    pub realtime: JackDaemonRealtimeConfig,

    /// Whether to disable audio reservation for the JACK server.
    /// If set to true, this will set the `JACK_NO_AUDIO_RESERVATION` environment variable to `1` when starting the JACK daemon.
    /// Defaults to `true`, which disables audio reservation.
    pub no_audio_reservation: bool,
}

impl Default for JackDaemonConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            period_size: 256,
            periods: 3,
            backend: "alsa".into(),
            capture_device: Some("hw:0".into()),
            playback_device: Some("hw:0".into()),
            realtime: JackDaemonRealtimeConfig::default(),
            no_audio_reservation: true,
        }
    }
}

impl JackDaemonConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.sample_rate == 0 {
            anyhow::bail!("Sample rate must be greater than 0");
        }

        if self.period_size == 0 || !self.period_size.is_power_of_two() {
            anyhow::bail!("Period size must be a power of two and greater than 0");
        }

        if self.periods < 2 {
            anyhow::bail!("At least 2 periods are required for the JACK server");
        }

        self.realtime.validate()?;

        Ok(())
    }
}

pub struct JackDaemon {
    child: Child,
}

impl JackDaemon {
    fn check_jack_lsp() -> anyhow::Result<bool> {
        let status = Command::new("jack_lsp")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        Ok(status.success())
    }

    pub fn start(config: &JackDaemonConfig) -> anyhow::Result<Self> {
        config.validate()?;

        let mut args = Vec::new();

        if config.realtime.enabled {
            args.push("-R".into());

            if let Some(priority) = config.realtime.priority {
                args.extend(["-P".into(), priority.to_string()]);
            }
        } else {
            args.push("-r".into());
        }

        args.extend(["-d".into(), config.backend.clone()]);

        if let Some(cd) = config.capture_device.as_ref() {
            args.extend(["-C".into(), cd.clone()]);
        }

        if let Some(pd) = config.playback_device.as_ref() {
            args.extend(["-P".into(), pd.clone()]);
        }

        args.extend([
            "-r".into(),
            config.sample_rate.to_string(),
            "-p".into(),
            config.period_size.to_string(),
            "-n".into(),
            config.periods.to_string(),
        ]);

        let mut child = Command::new("jackd")
            .args(&args)
            .env(
                "JACK_NO_AUDIO_RESERVATION",
                if config.no_audio_reservation {
                    "1"
                } else {
                    "0"
                },
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        let timeout = Duration::from_secs(10);
        let start = Instant::now();

        loop {
            if let Some(status) = child.try_wait()? {
                anyhow::bail!("JACK daemon exited unexpectedly with status: {}", status);
            }

            if Self::check_jack_lsp()? {
                return Ok(Self { child });
            }

            if start.elapsed() >= timeout {
                _ = child.kill();
                _ = child.wait();

                break;
            }

            thread::sleep(Duration::from_millis(100));
        }

        anyhow::bail!("Timed out waiting for JACK daemon to start");
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn stop(&mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
        }

        self.child.wait()?;
        Ok(())
    }
}

impl Drop for JackDaemon {
    fn drop(&mut self) {
        _ = self.stop();
    }
}
