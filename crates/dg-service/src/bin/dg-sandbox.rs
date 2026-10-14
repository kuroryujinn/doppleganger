fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = if args == ["--probe"] {
        dg_service::sandbox::probe().map(|p| {
            println!("{p}");
        })
    } else if args.is_empty() {
        dg_service::sandbox::run()
    } else {
        Err(dg_service::ServiceError::new(
            "INVALID_ARGUMENTS",
            "dg-sandbox accepts only --probe or no arguments",
        ))
    };
    if let Err(error) = result {
        eprintln!("{}", error.code);
        std::process::exit(1);
    }
}
