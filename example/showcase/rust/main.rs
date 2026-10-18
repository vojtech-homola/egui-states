fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut port = 8091;
    let mut web = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                port = args
                    .next()
                    .ok_or("--port requires a number")?
                    .parse::<u16>()?
            }
            "--web" => web = Some(args.next().ok_or("--web requires a directory")?),
            _ => return Err("Usage: showcase-server [--port PORT] [--web DIR]".into()),
        }
    }
    let demo = showcase_server::setup_server()?;
    if let Some(web) = web {
        demo.server.serve_web(web)?;
    }
    demo.server
        .start(port, Some(std::net::Ipv4Addr::LOCALHOST), None)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        // Register synchronously so Ctrl+C is handled as soon as readiness is printed.
        #[cfg(unix)]
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        #[cfg(windows)]
        let mut interrupt = tokio::signal::windows::ctrl_c()?;

        println!("Showcase server listening on {port}");
        interrupt.recv().await;
        Ok::<_, std::io::Error>(())
    })?;
    demo.server.stop();
    Ok(())
}
