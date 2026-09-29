//! Experimental: plays through a sound server forwarded from your SSH client.
//! Built only with `--features remote`; see docs/remote-audio.md.

fn main() -> anyhow::Result<()> {
    tidally::run(tidally::Edition::Remote)
}
