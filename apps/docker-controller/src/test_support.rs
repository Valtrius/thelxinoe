use crate::docker::{Result, unavailable};
use axum::http::StatusCode;
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Fault {
    CreateReply,
    CreateReplyAndInspection,
    JournalWrite,
    JournalBefore(&'static str),
    JournalAfter(&'static str),
    StartBefore(&'static str),
    StartReply(&'static str),
}
struct Engine {
    containers: BTreeMap<String, Value>,
    fault: Option<Fault>,
    disconnected: bool,
    creates: usize,
    deletes: usize,
    copies: usize,
    starts: usize,
}
pub(crate) struct Docker {
    root: tempfile::TempDir,
    engine: Arc<Mutex<Engine>>,
}
tokio::task_local! {
    pub(crate) static DOCKER: DockerContext;
}
#[derive(Clone)]
pub(crate) struct DockerContext {
    pub root: std::path::PathBuf,
    engine: Arc<Mutex<Engine>>,
}
impl Docker {
    pub(crate) fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("appdata");
        std::fs::create_dir(&source).unwrap();
        let original = json!({"Id":"original","Name":"/service","Image":"old-image","State":{"Running":false,"StartedAt":"0001-01-01T00:00:00Z"},"Config":{"Image":"old-image","Labels":{"app.thelxinoe.deployment":"deployment","app.thelxinoe.managed-id":"11111111-1111-1111-1111-111111111111"}},"Mounts":[{"Destination":"/config","Source":source}],"HostConfig":{},"NetworkSettings":{"Networks":{"media":{}}}});
        Self {
            root,
            engine: Arc::new(Mutex::new(Engine {
                containers: BTreeMap::from([("original".into(), original)]),
                fault: None,
                disconnected: false,
                creates: 0,
                deletes: 0,
                copies: 0,
                starts: 0,
            })),
        }
    }
    pub(crate) async fn scope<F: std::future::Future>(&self, future: F) -> F::Output {
        DOCKER
            .scope(
                DockerContext {
                    root: self.root.path().to_owned(),
                    engine: self.engine.clone(),
                },
                future,
            )
            .await
    }
    pub(crate) fn update<D: DeserializeOwned, U: DeserializeOwned>(&self) -> (D, U) {
        let old = self.engine.lock().unwrap().containers["original"].clone();
        (serde_json::from_value(json!({"id":"deployment","generation":1,"version":"test","server":{},"controller":{},"network":"media","media_source":"/media","appdata_source":"/state"})).unwrap(),
        serde_json::from_value(json!({"id":"22222222-2222-2222-2222-222222222222","service":"11111111-1111-1111-1111-111111111111","candidate":"candidate-image","stage":"isolated-live-validation","classification":"compatible","error":null,"old":{"id":"11111111-1111-1111-1111-111111111111","kind":"radarr","container":"original","name":"service","image":"old-image","phase":"active","spec":{"Image":"old-image","Labels":old["Config"]["Labels"],"HostConfig":{}},"expected":crate::policy::fingerprint(&old)},"was_running":true,"snapshot_complete":true,"recovery_complete":false,"activation_crossed":false,"candidate_container":null,"replacement":null})).unwrap())
    }
    pub(crate) fn fault(&self, fault: Fault) {
        self.engine.lock().unwrap().fault = Some(fault);
    }
    pub(crate) fn backup_deployment<D: DeserializeOwned>(&self) -> D {
        let mut engine = self.engine.lock().unwrap();
        let server = engine.containers.get_mut("original").unwrap();
        server["Mounts"][0]["Destination"] = json!("/var/lib/thelxinoe");
        serde_json::from_value(json!({"id":"deployment","generation":1,"version":"test","server":server,"controller":{"Image":"controller-image"},"network":"media","media_source":"/media","appdata_source":self.root.path()})).unwrap()
    }
    pub(crate) fn root(&self) -> &std::path::Path {
        self.root.path()
    }
    pub(crate) fn copies(&self) -> usize {
        self.engine.lock().unwrap().copies
    }
    pub(crate) fn add_component(&self, id: &str) -> String {
        let source = self.root.path().join(id);
        std::fs::create_dir(&source).unwrap();
        let mut e = self.engine.lock().unwrap();
        let mut raw = e.containers["original"].clone();
        raw["Id"] = json!(id);
        raw["Name"] = json!(format!("/{id}"));
        raw["Mounts"][0]["Source"] = json!(source);
        e.containers.insert(id.into(), raw);
        source.to_string_lossy().into_owned()
    }
    pub(crate) fn reconnect(&self) {
        let mut e = self.engine.lock().unwrap();
        e.disconnected = false;
        e.fault = None;
    }
    pub(crate) fn names(&self) -> Vec<String> {
        self.engine
            .lock()
            .unwrap()
            .containers
            .values()
            .map(|c| c["Name"].as_str().unwrap().into())
            .collect()
    }
    pub(crate) fn running(&self, id: &str) -> bool {
        self.engine.lock().unwrap().containers[id]["State"]["Running"] == true
    }
    pub(crate) fn creates(&self) -> usize {
        self.engine.lock().unwrap().creates
    }
    pub(crate) fn deletes(&self) -> usize {
        self.engine.lock().unwrap().deletes
    }
    pub(crate) fn change_replacement(&self, spec: bool) {
        let mut e = self.engine.lock().unwrap();
        let c = e.containers.get_mut("replacement").unwrap();
        if spec {
            c["Config"]["Image"] = json!("foreign-image");
        } else {
            c["Config"]["Labels"] = json!({});
        }
    }
}
impl DockerContext {
    pub(crate) fn fail_write(&self, bytes: &[u8]) -> bool {
        let mut e = self.engine.lock().unwrap();
        if e.fault == Some(Fault::JournalWrite)
            && serde_json::from_slice::<Value>(bytes)
                .is_ok_and(|v| v["stage"] == "replacement-created")
        {
            e.fault = None;
            return true;
        }
        if let Some(Fault::JournalBefore(stage)) = e.fault
            && serde_json::from_slice::<Value>(bytes).is_ok_and(|v| v["stage"] == stage)
        {
            e.fault = None;
            return true;
        }
        false
    }
    pub(crate) fn fail_after_write(&self, bytes: &[u8]) -> bool {
        let mut e = self.engine.lock().unwrap();
        if let Some(Fault::JournalAfter(stage)) = e.fault
            && serde_json::from_slice::<Value>(bytes).is_ok_and(|v| v["stage"] == stage)
        {
            e.fault = None;
            return true;
        }
        false
    }
    pub(crate) fn request(
        &self,
        method: &Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        let mut e = self.engine.lock().unwrap();
        if e.disconnected {
            return Err(unavailable());
        }
        if path == "/containers/json?all=true" {
            return Ok(Value::Array(e.containers.values().map(|c|json!({"Id":c["Id"],"Names":[c["Name"]],"Labels":c["Config"]["Labels"]})).collect()));
        }
        if path.starts_with("/images/") {
            return Ok(json!({"Config":{}}));
        }
        if path == "/containers/controller-fixture/json" {
            return Ok(json!({"Image":format!("sha256:{}", "a".repeat(64))}));
        }
        if let Some(name) = path.strip_prefix("/containers/create?name=") {
            assert_eq!(method, Method::POST);
            let spec = body.unwrap();
            e.creates += 1;
            assert!(
                !e.containers
                    .values()
                    .any(|c| c["Name"] == format!("/{name}"))
            );
            let mut config = spec.clone();
            config.as_object_mut().unwrap().remove("HostConfig");
            let id = if spec["Cmd"][0]
                .as_str()
                .is_some_and(|c| c.starts_with("snapshot-"))
            {
                format!("worker-{}", e.creates)
            } else {
                "replacement".into()
            };
            e.containers.insert(id.clone(),json!({"Id":id,"Name":format!("/{name}"),"Image":spec["Image"],"Config":config,"HostConfig":spec["HostConfig"],"State":{"Running":false,"StartedAt":"0001-01-01T00:00:00Z"},"NetworkSettings":{"Networks":{"media":{}}}}));
            match e.fault {
                Some(Fault::CreateReplyAndInspection) => {
                    e.disconnected = true;
                    return Err(unavailable());
                }
                Some(Fault::CreateReply) => {
                    e.fault = None;
                    return Err(unavailable());
                }
                _ => {}
            }
            return Ok(json!({"Id":id}));
        }
        let tail = path
            .strip_prefix("/containers/")
            .expect("unexpected Docker route");
        let (id, action) = tail
            .split_once('/')
            .unwrap_or((tail.split('?').next().unwrap(), ""));
        if method == Method::DELETE {
            e.deletes += 1;
            e.containers.remove(id);
            return Ok(Value::Null);
        }
        if !e.containers.contains_key(id) {
            return Err((StatusCode::NOT_FOUND, "missing"));
        }
        if action == "json" {
            return Ok(e.containers[id].clone());
        }
        if let Some(name) = action.strip_prefix("rename?name=") {
            if e.containers
                .iter()
                .any(|(key, c)| key != id && c["Name"] == format!("/{name}"))
            {
                return Err(unavailable());
            }
            e.containers.get_mut(id).unwrap()["Name"] = json!(format!("/{name}"));
        } else if action.starts_with("stop") {
            e.containers.get_mut(id).unwrap()["State"]["Running"] = json!(false);
        } else if action == "start" {
            if matches!(e.fault,Some(Fault::StartBefore(target)) if target == id) {
                e.fault = None;
                return Err(unavailable());
            }
            e.starts += 1;
            let started = format!("2026-10-03T22:00:{:02}Z", e.starts);
            e.containers.get_mut(id).unwrap()["State"]["StartedAt"] = json!(started);
            e.containers.get_mut(id).unwrap()["State"]["Running"] = json!(true);
            let raw = e.containers[id].clone();
            if raw["Config"]["Cmd"][0]
                .as_str()
                .is_some_and(|c| c.starts_with("snapshot-"))
            {
                let mounts = raw["HostConfig"]["Mounts"].as_array().unwrap();
                let source = std::path::Path::new(mounts[0]["Source"].as_str().unwrap());
                let destination = std::path::Path::new(mounts[1]["Source"].as_str().unwrap());
                assert!(source.starts_with(&self.root) && destination.starts_with(&self.root));
                std::fs::create_dir_all(destination).unwrap();
                std::fs::copy(source.join("marker"), destination.join("marker")).unwrap();
                e.copies += 1;
                e.containers.get_mut(id).unwrap()["State"]["Running"] = json!(false);
                e.containers.get_mut(id).unwrap()["State"]["ExitCode"] = json!(0);
            }
            if let Some(source) = raw["Mounts"][0]["Source"].as_str() {
                std::fs::write(
                    std::path::Path::new(source).join("marker"),
                    "production-after-start",
                )
                .unwrap();
            }
            if matches!(e.fault,Some(Fault::StartReply(target)) if target == id) {
                e.fault = None;
                return Err(unavailable());
            }
        } else {
            panic!("Unexpected Docker operation {method} {path}");
        }
        Ok(Value::Null)
    }
}
