pub mod templates;
mod utils;

use std::collections::HashMap;
use openapiv3::{Content, MediaType, OpenAPI, Operation, Paths, ReferenceOr, RequestBody};
use serde::{Deserialize, Serialize};
use tera::{Context, Tera};

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
			let v = content.iter().filter(|(k, _)| k.contains("json")).next().unwrap_or(content.iter().next().expect("No content"));
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
			Some(v) => v.clone()
		};
		let body = operation.request_body.as_ref().and_then(|v| {
			let request_body = v.as_item().expect("Referenced request body is not allowd");
			//jsonを含めば、json、それ以外ならbytes
			Some(LappedRequestBody {
				request_body: request_body.clone(),
				default_content: LappedMediaType::try_from(&request_body.content).unwrap(),
				identifier: utils::camel_case(format!("request_body_{}", function.as_str()).as_str()),
			})
		});
		Self {
			path: path.to_string(),
			method: method.to_string(),
			operation: operation.clone(),
			function: function.clone(),
			request_identifier: utils::camel_case(format!("request_{}", function.as_str()).as_str()),
			response_identifier: utils::camel_case(format!("response_{}", function.as_str()).as_str()),
			request_body: body,
		}
	}
}

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
	pub fn template<T: AsRef<str>>(&mut self, template: T) -> &Self {
		self.templates.push(template.as_ref().to_string());
		self
	}
	pub fn render(&self) -> Result<String, tera::Error> {
		let mut tera = Tera::default();
		let json = serde_json::to_value(&self.api)?;
		// 空の辞書を返す関数
		tera.register_function("m", |_: &HashMap<String, tera::Value>| {
			Ok(tera::Value::Object(Default::default()))
		});
		// JsonPointer文字列を実態に変換するフィルター
		let json_for_p=json.clone();
		tera.register_filter("p", move |value: &tera::Value, _: &HashMap<String, tera::Value>| {
			let default = Ok(tera::Value::Object(Default::default()));
			let path = value.as_str().unwrap_or_default();
			let mut parent = &json_for_p;
			for p in path.split("/").skip(1) {
				let p=p.replace("~0","~").replace("~1","/");// RFC6901
				parent = if let serde_json::Value::Object(map) = parent {
					match map.get(p.as_str()) {
						None => return default,
						Some(latest) => latest,
					}
				} else if let serde_json::Value::Array(array) = parent {
					match p.parse::<usize>() {
						Ok(v) => match array.get(v) {
							None => return default,
							Some(latest) => latest,
						},
						Err(_) => return default,
					}
				} else {
					return default;
				}
			}
			return Ok(parent.clone());
		});
		tera.register_filter("ref", |value: &tera::Value, _: &HashMap<String, tera::Value>| {
			let i = tera::try_get_value!("ref", "value", ReferenceOr<()>, value);
			let v = match i {
				ReferenceOr::Reference { reference } => reference.replace("#/components/schemas/", "").to_string(),
				ReferenceOr::Item(_) => "".to_string(),
			};
			Ok(tera::to_value(v).unwrap())
		});
		tera.register_filter("content_into_media", |value: &tera::Value, _: &HashMap<String, tera::Value>| {
			let i = tera::try_get_value!("content_into_media", "value", Content, value);
			LappedMediaType::try_from(&i)
				.map(|v| tera::to_value(v).unwrap())
				.map_err(|_| tera::Error::from("content_into_media: no content"))
		});
		tera.register_filter("paths_into_operations", |value: &tera::Value, _: &HashMap<String, tera::Value>| {
			let paths = tera::try_get_value!("paths_into_operations", "value", Paths, value);
			let operations: Vec<LappedOperation> = paths.iter()
				.filter(|(_, path)| path.as_item().is_some())
				.map(|(path_key, path)| {
					let path = path.as_item().unwrap();
					[
						("get", path.get.clone()),
						("delete", path.delete.clone()),
						("head", path.head.clone()),
						("options", path.options.clone()),
						("patch", path.patch.clone()),
						("post", path.post.clone()),
						("put", path.put.clone()),
						("trace", path.trace.clone()),
					]
						.into_iter()
						.filter(|(_method, operation)| operation.is_some())
						.map(|(method, operation)| LappedOperation::new(path_key, method, &operation.unwrap()))
				})
				.flatten()
				.collect();
			tera::to_value(operations).map_err(|e| tera::Error::from(e.to_string()))
		});
		let context = Context::from_serialize(&self.api)?;
		tera.render_str(self.templates.join("\n").as_str(), &context)
	}
}
#[cfg(test)]
mod tests {
	use std::fs;
	use std::fs::File;
	use std::io::BufReader;
	use std::path::Path;
	use super::*;
	fn apis()->HashMap<String, OpenAPI>{
		fs::read_dir(&Path::new(".").join("openapi")).unwrap()
			.filter_map(Result::ok)
			.filter_map(|entry|
				entry.path().to_str().unwrap_or_default().contains("yaml").then(||
					(
						entry.file_name().to_str().unwrap_or_default().to_string(),
						serde_yaml::from_reader(BufReader::new(File::open(entry.path()).unwrap())).unwrap()
					)
				)
			)
			.collect()
	}
	#[test]
	fn test_filter() {
		let v=apis().get("openapi.yaml").unwrap().clone();
		let r = Mandolin::new(v)
			.template("{{'#'|p|json_encode()}}\n{{'#/paths'|p|json_encode()}}\n{{'#/servers/0'|p|json_encode()}}")
			.render()
			.unwrap();
		println!("{}", r)
	}
	#[test]
	fn test_render() {
		for entry in fs::read_dir(&Path::new(".").join("openapi")).unwrap().filter_map(Result::ok) {
			if entry.path().extension().unwrap_or_default().to_str().unwrap_or_default().contains("yaml") {
				let v = Mandolin::new(serde_yaml::from_reader(BufReader::new(File::open(entry.path()).unwrap())).unwrap())
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