pub fn run(args: noargs::RawArgs) -> noargs::Result<()> {
    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    todo!()
}
