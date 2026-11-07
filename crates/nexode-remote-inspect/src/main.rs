fn main() -> anyhow::Result<()> {
    nexode_remote_inspect::run(std::env::args_os().skip(1))
}
