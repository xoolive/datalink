use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "acars131", about = "Classic VHF ACARS frontend")]
struct Args {
    #[arg(short, long, help = "Path to I/Q recording file")]
    file: Option<String>,
    #[arg(
        long,
        num_args = 1..,
        default_values_t = [131_525_000u32, 131_725_000u32, 131_825_000u32],
        help = "ACARS channel frequency (or frequencies) to decode in Hz"
    )]
    channel: Vec<u32>,
}

fn main() -> anyhow::Result<()> {
    let _ = Args::parse();
    anyhow::bail!("acars131 frontend is planned but not implemented yet")
}
