pub mod templates;
mod utils;

use openapiv3::{Content, MediaType, OpenAPI, Operation, ReferenceOr, RequestBody};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

#[derive(Serialize, Deserialize)]
pub struct LappedMediaType {
    pub media_type: String,
    #[serde(flatten)]
    pub media: MediaType,
}
impl TryFrom<&Content> for LappedMediaType {
    type Error = ();
    fn try_from(content: &Content) -> Result<Self, Self::Error> {
        if content.is_empty() {
            Err(())
        } else {
            let v = content
                .iter()
                .filter(|(k, _)| k.contains("json"))
                .next()
                .unwrap_or(content.iter().next().expect("No content"));
            Ok(Self {
                media_type: v.0.to_string(),
                media: v.1.clone(),
            })
        }
    }
}
#[derive(Serialize, Deserialize)]
pub struct LappedRequestBody {
    pub default_content: LappedMediaType,
    pub identifier: String,
    #[serde(flatten)]
    pub request_body: RequestBody,
    //required
    //content
}
#[derive(Serialize, Deserialize)]
pub struct LappedOperation {
    pub path: String,
    pub method: String,
    pub function: String,
    pub request_identifier: String,
    pub response_identifier: String,
    //Option<ReferenceOr<RequestBody>>だがReference<>を許容しないのでOption<RequestBody>
    //Option<RequestBody>だがcontext_oneを追加してOption<LappedRequestBody>
    pub request_body: Option<LappedRequestBody>,
    #[serde(flatten)]
    pub operation: Operation,
}
impl LappedOperation {
    pub fn new(path: &str, method: &str, operation: &Operation) -> Self {
        let function = match &operation.operation_id {
            None => format!("{}{}", method, path).replace("/", "_"),
            Some(v) => v.clone(),
        };
        let body = operation.request_body.as_ref().and_then(|v| {
            let request_body = v.as_item().expect("Referenced request body is not allowd");
            //jsonを含めば、json、それ以外ならbytes
            Some(LappedRequestBody {
                request_body: request_body.clone(),
                default_content: LappedMediaType::try_from(&request_body.content).unwrap(),
                identifier: utils::camel_case(
                    format!("request_body_{}", function.as_str()).as_str(),
                ),
            })
        });
        Self {
            path: path.to_string(),
            method: method.to_string(),
            operation: operation.clone(),
            function: function.clone(),
            request_identifier: utils::camel_case(
                format!("request_{}", function.as_str()).as_str(),
            ),
            response_identifier: utils::camel_case(
                format!("response_{}", function.as_str()).as_str(),
            ),
            request_body: body,
        }
    }
}
#[derive(Serialize, Deserialize)]
pub struct Empty {}
static EMPTY_OBJECT: LazyLock<minijinja::Value> =
    LazyLock::new(|| minijinja::Value::from_serialize(Empty {}));
static EMPTY_ARRAY: LazyLock<minijinja::Value> =
    LazyLock::new(|| minijinja::Value::from_serialize::<[(); 0]>([(); 0]));

pub struct Mandolin {
    api: OpenAPI,
    templates: Vec<String>,
}
impl Mandolin {
    pub fn new(api: OpenAPI) -> Self {
        Self {
            api,
            templates: vec![],
        }
    }
    pub fn template<T: AsRef<str>>(&mut self, template: T) -> &mut Self {
        self.templates.push(template.as_ref().to_string());
        self
    }
    fn p(
        api: minijinja::Value,
        path: &str,
        no_err: bool,
    ) -> Result<minijinja::Value, minijinja::Error> {
        let default = if no_err {
            Ok(EMPTY_OBJECT.clone())
        } else {
            Err(minijinja::Error::new(
                minijinja::ErrorKind::NonKey,
                format!("p: {path} not found"),
            ))
        };
        let mut parent = api;
        for p in path.split("/").skip(1) {
            let p = p.replace("~0", "~").replace("~1", "/"); // RFC6901
            parent = if let Some(map) = parent.as_object() {
                match map.get_value(&minijinja::Value::from(&p)) {
                    None => match p
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| map.get_value(&minijinja::Value::from(i)))
                    {
                        None => return default,
                        Some(latest) => latest,
                    },
                    Some(latest) => latest,
                }
            } else {
                return default;
            }
        }
        Ok(parent)
    }
    fn r<'a>(
        api: minijinja::Value,
        value: minijinja::Value,
        no_err: bool,
    ) -> Result<minijinja::Value, minijinja::Error> {
        match value.downcast_object_ref::<ReferenceOr<minijinja::Value>>() {
            Some(ReferenceOr::Reference { reference }) => Self::p(api, reference.as_str(), no_err),
            _ => Ok(value),
        }
    }
    fn pr<'a>(
        api: minijinja::Value,
        path: &str,
        no_err: bool,
    ) -> Result<minijinja::Value, minijinja::Error> {
        let v = Self::p(api.clone(), path, no_err)?;
        Self::r(api, v, false)
    }
    fn ls(
        api: minijinja::Value,
        path: &str,
        no_err: bool,
    ) -> Result<Vec<(String, minijinja::Value)>, minijinja::Error> {
        let v = Self::pr(api, path, no_err)?;
        if let Some(v) = v.as_object() {
            if let Some(v) = v.try_iter_pairs() {
                return Ok(v.map(|(k, v)| (format!("{path}/{}", k.to_string().replace("~", "~0").replace("/", "~1")), v,)).collect())
            } else if let Some(v) = v.try_iter() {
                return Ok(v.enumerate().map(|(k, v)| (format!("{path}/{}", k), v)).collect())
            }
        }
        if no_err {
            Ok(Default::default())
        } else {
            Err(minijinja::Error::new(
                minijinja::ErrorKind::NonKey,
                format!("ls {}", path),
            ))
        }
    }
    fn lsop(
        api: minijinja::Value,
        path: &str,
        no_err: bool,
    ) -> Result<Vec<(String, minijinja::Value)>, minijinja::Error> {
        let v = Self::ls(api.clone(), path, no_err)?;
        let methods = [
            "get", "put", "post", "delete", "options", "head", "patch", "trace",
        ];
        let w = v
            .iter()
            .map(|(k, v_path)| {
                println!("{}", k);
                Self::ls(api.clone(), k.as_str(), no_err)
                    .unwrap_or_default()
                    .into_iter()
                    .map(move |(k, v)| (k, v_path, v))
            })
            .flatten()
            .filter(|(k, _, _)| methods.iter().any(|v| k.ends_with(v)))
            .map(|(k, _, w)| (k, w)) // 本来はここで親元のparametersを合成することができるはず
            .collect();
        Ok(w)
    }
    pub fn render(&self) -> Result<String, minijinja::Error> {
        let mut env = minijinja::Environment::new();
        let api = minijinja::Value::from_serialize(&self.api);
        env.add_filter("json_encode", minijinja::filters::tojson);
        env.add_function("m", || { Ok(minijinja::Value::from_serialize(Empty{})) });
        {
            let api = api.clone();
            env.add_filter(
                "p",
                move |value: &minijinja::Value| {
                    Self::p(api.clone(), value.as_str().unwrap_or_default(), true)
                },
            );
        }
        {
            let api = api.clone();
            env.add_filter(
                "r",
                move |value: minijinja::Value| {
                    Self::r(api.clone(), value, false)
                },
            );
        }
        {
            let api = api.clone();
            env.add_filter(
                "pr",
                move |value: &minijinja::Value| {
                    Self::pr(api.clone(), value.as_str().unwrap_or_default(), true)
                },
            );
        }
        {
            let api = api.clone();
            env.add_filter(
                "ls",
                move |value: &minijinja::Value| {
                    Self::ls(api.clone(), value.as_str().unwrap_or_default(), true).map(|v| minijinja::Value::from_serialize(v))
                },
            );
        }
        {
            let api = api.clone();
            env.add_filter(
                "lsop",
                move |value: &minijinja::Value| {
                    Self::lsop(api.clone(), value.as_str().unwrap_or_default(), true).map(|v| minijinja::Value::from_serialize(v))
                },
            );
        }
        let v=self.templates.join("\n");
        env.add_template("main", v.as_str())?;
        let template = env.get_template("main")?;
        template.render(&self.api)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::fs::File;
    use std::io::BufReader;
    use std::path::Path;
    fn apis() -> HashMap<String, OpenAPI> {
        fs::read_dir(&Path::new(".").join("openapi"))
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry
                    .path()
                    .to_str()
                    .unwrap_or_default()
                    .contains("yaml")
                    .then(|| {
                        (
                            entry.file_name().to_str().unwrap_or_default().to_string(),
                            serde_yaml::from_reader(BufReader::new(
                                File::open(entry.path()).unwrap(),
                            ))
                            .unwrap(),
                        )
                    })
            })
            .collect()
    }
    #[test]
    fn test_filter() {
        let v = apis().get("openapi.yaml").unwrap().clone();
        let r = Mandolin::new(v)
            .template("{{'#'|p|json_encode}}\n{{'#/paths'|p|json_encode}}\n{{'#/servers/0'|p|json_encode}}\n{{'#'|ls|json_encode}}{{'#/servers'|ls|json_encode}}\n{{'#/paths'|lsop|json_encode}}")
            .render()
            .unwrap();
        println!("{}", r)
    }
    #[test]
    fn test_ls() {
        let v = apis().get("openapi.yaml").unwrap().clone();
        let r = Mandolin::new(v)
            .template("{% for k, v in '#'|ls %}{{k}}={{v}}\n{%endfor%}")
            .render()
            .unwrap();
        println!("{}", r)
    }
    #[test]
    fn test_render() {
        for entry in fs::read_dir(&Path::new(".").join("openapi"))
            .unwrap()
            .filter_map(Result::ok)
        {
            if entry
                .path()
                .extension()
                .unwrap_or_default()
                .to_str()
                .unwrap_or_default()
                .contains("yaml")
            {
                let v = Mandolin::new(
                    serde_yaml::from_reader(BufReader::new(File::open(entry.path()).unwrap()))
                        .unwrap(),
                )
                .template(templates::MAIN)
                .render()
                .unwrap();
                println!("{}", v)
            }
        }
    }
    #[test]
    fn test_camel_case() {
        println!("{}", utils::camel_case("abc_def"))
    }
}
