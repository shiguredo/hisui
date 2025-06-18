use crate::{command_line_args::Args, runner::Runner};

use orfail::OrFail;

pub fn run(args: noargs::RawArgs) -> noargs::Result<()> {
    let args = Args::parse(args)?;
    if let Some(help) = args.get_help() {
        print!("{help}");
        return Ok(());
    }
    Runner::new(args).run().or_fail()?;
    Ok(())
}
