//! Consistent diagnostics for embedded Python converters.
use crate::Error;
use std::process::Command;

pub(crate) fn script(source: &str, label: &str, dependencies: &str) -> Result<String, Error> {
    Ok(format!(
        "import os,sys,traceback\nsource={}\nlabel={}\nrequirements={}\ntry:\n exec(compile(source, '<rusty-tiles/' + label + '.py>', 'exec'), globals())\nexcept ImportError as error:\n print(label + ': Python dependency import failed: ' + str(error) + '; install ' + requirements + '; run rusty-tiles doctor', file=sys.stderr)\n if os.environ.get('RUSTY_TILES_PYTHON_TRACEBACK') == '1': traceback.print_exc()\n sys.exit(1)\nexcept Exception as error:\n print(label + ': ' + str(error), file=sys.stderr)\n if os.environ.get('RUSTY_TILES_PYTHON_TRACEBACK') == '1': traceback.print_exc()\n sys.exit(1)\n",
        serde_json::to_string(source)?,
        serde_json::to_string(label)?,
        serde_json::to_string(dependencies)?,
    ))
}

pub(crate) fn run(command: &mut Command, label: &str) -> Result<(), Error> {
    let status = command.status().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg(format!("Python 3 executable is required for {label}"))
        } else {
            Error::Io(error)
        }
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::msg(format!(
            "{label} conversion failed; no output published."
        )))
    }
}
