use agentix_harness::Tool;
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

pub struct RunCommand {
    working_dir: PathBuf,
}

impl RunCommand {
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        Self {
            working_dir: working_dir.into(),
        }
    }
}

#[async_trait]
impl Tool for RunCommand {
    fn name(&self) -> &str {
        "run_command"
    }

    fn description(&self) -> &str {
        "Execute a shell command (via sh -c) in the project working directory. \
         Returns stdout and stderr combined. Times out after 60 seconds."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to run (passed to sh -c)."
                }
            },
            "required": ["command"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> Result<String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing command"))?;

        let child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let result = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output()).await;

        match result {
            Err(_) => Ok("error: command timed out after 60s".into()),
            Ok(Err(e)) => Ok(format!("error: {e}")),
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let code = output.status.code().unwrap_or(-1);

                let mut parts = vec![];
                if !stdout.is_empty() {
                    parts.push(format!("stdout:\n{}", stdout.trim_end()));
                }
                if !stderr.is_empty() {
                    parts.push(format!("stderr:\n{}", stderr.trim_end()));
                }
                parts.push(format!("exit code: {code}"));
                Ok(parts.join("\n\n"))
            }
        }
    }
}
