//! Consistent diagnostics and private error IPC for embedded Python converters.
use crate::Error;
use std::process::Command;

pub(crate) fn script(source: &str, label: &str, dependencies: &str) -> Result<String, Error> {
    let wrapper = r#"if os.environ.get('RUSTY_TILES_JSON_STDOUT') == '1': sys.stdout=sys.stderr
def failure(error, code, message):
 path=os.environ.get('RUSTY_TILES_RESULT_FILE')
 if path:
  with open(path,'w') as result: json.dump(dict(code=code,message=message),result)
 print(message,file=sys.stderr)
 if os.environ.get('RUSTY_TILES_PYTHON_TRACEBACK') == '1': traceback.print_exc()
 sys.exit(code)
try:
 exec(compile(source, '<rusty-tiles/' + label + '.py>', 'exec'), globals())
except ImportError as error:
 failure(error,4,label + ': Python dependency import failed: ' + str(error) + '; install ' + requirements + '; run rusty-tiles doctor')
except EnvironmentError as error:
 failure(error,1,label + ': ' + str(error))
except Exception as error:
 failure(error,4 if getattr(error,'environment_error',False) else 3,label + ': ' + str(error))
"#;
    Ok(format!(
        "import os,sys,traceback,json\nsource={}\nlabel={}\nrequirements={}\n{}",
        serde_json::to_string(source)?,
        serde_json::to_string(label)?,
        serde_json::to_string(dependencies)?,
        wrapper
    ))
}

pub(crate) fn run(command: &mut Command, label: &str) -> Result<(), Error> {
    let result_file = tempfile::NamedTempFile::new()?;
    command.env("RUSTY_TILES_RESULT_FILE", result_file.path());
    let status = command.status().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::Environment(format!("Python 3 executable is required for {label}"))
        } else {
            Error::Io(error)
        }
    })?;
    if status.success() {
        return Ok(());
    }
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(result_file.path())?).unwrap_or_default();
    let message = report["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{label} conversion failed; no output published."));
    match status.code() {
        Some(4) => Err(Error::Environment(message)),
        Some(1) => Err(Error::Io(std::io::Error::other(message))),
        _ => Err(Error::Data(message)),
    }
}
