use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "xtask", about = "Project automation tasks", version)]
struct Args {
    #[arg(long)]
    task: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    println!("xtask scaffold: {:?}", args.task);
    Ok(())
}
