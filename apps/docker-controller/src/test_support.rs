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
}
struct Engine {
    containers: BTreeMap<String, Value>,
    fault: Option<Fault>,
    disconnected: bool,
    creates: usize,
    deletes: usize,
}
pub(crate) struct Docker {
    root: tempfile::TempDir,
    engine: Arc<Mutex<Engine>>,
}
tokio::task_local! {
    pub(crate) static DOCKER: DockerContext;
}
pub(crate) struct DockerContext {
    pub root: std::path::PathBuf,
    engine: Arc<Mutex<Engine>>,
}
impl Docker {
    pub(crate) fn new() -> Self {
        let original = json!({"Id":"original","Name":"/service","Image":"old-image","State":{"Running":false,"StartedAt":"0001-01-01T00:00:00Z"},"Config":{"Image":"old-image","Labels":{"app.thelxinoe.deployment":"deployment","app.thelxinoe.managed-id":"11111111-1111-1111-1111-111111111111"}},"HostConfig":{},"NetworkSettings":{"Networks":{"media":{}}}});
        Self {
            root: tempfile::tempdir().unwrap(),
            engine: Arc::new(Mutex::new(Engine {
                containers: BTreeMap::from([("original".into(), original)]),
                fault: None,
                disconnected: false,
                creates: 0,
                deletes: 0,
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
            e.containers.insert("replacement".into(),json!({"Id":"replacement","Name":format!("/{name}"),"Image":spec["Image"],"Config":config,"HostConfig":spec["HostConfig"],"State":{"Running":false,"StartedAt":"0001-01-01T00:00:00Z"},"NetworkSettings":{"Networks":{"media":{}}}}));
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
            return Ok(json!({"Id":"replacement"}));
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
            e.containers.get_mut(id).unwrap()["State"]["Running"] = json!(true);
        } else {
            panic!("Unexpected Docker operation {method} {path}");
        }
        Ok(Value::Null)
    }
}
