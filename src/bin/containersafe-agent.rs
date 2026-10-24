use containersafe::agent::{Agent, ErrorCode, Response, MAX_REQUEST_BYTES, PROTOCOL_VERSION};
use std::{
    io::{self, Read, Write},
    path::Path,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        println!("ContainerSafe Agent {}", containersafe::VERSION);
        return;
    }
    if args == ["--capabilities"] {
        println!(
            "{}",
            serde_json::json!({"protocol_version": PROTOCOL_VERSION,
            "application_version": containersafe::VERSION, "platform_supported": cfg!(target_os = "linux"),
            "transport": "stdio-one-request", "operations": ["init", "backup", "snapshots", "check", "restore"]})
        );
        return;
    }
    let response = match args.as_slice() {
        [option, config] if option == "--config" => {
            let mut input = Vec::new();
            if io::stdin()
                .take(MAX_REQUEST_BYTES + 1)
                .read_to_end(&mut input)
                .is_err()
            {
                Response::failure(None, ErrorCode::InvalidRequest)
            } else {
                match containersafe::agent::parse_request(&input) {
                    Err(error) => Response::failure(None, error),
                    Ok(request) => match Agent::load(Path::new(config)) {
                        Err(error) => Response::failure(Some(request.request_id), error),
                        Ok(agent) => agent.handle(request),
                    },
                }
            }
        }
        _ => {
            eprintln!("Usage: containersafe-agent --config ABSOLUTE_PATH < request.json\n       containersafe-agent --capabilities | --version");
            std::process::exit(2);
        }
    };
    let mut stdout = io::stdout().lock();
    if serde_json::to_writer(&mut stdout, &response).is_err()
        || writeln!(stdout).is_err()
        || stdout.flush().is_err()
    {
        std::process::exit(2);
    }
    if !response.is_success() {
        std::process::exit(1);
    }
}
