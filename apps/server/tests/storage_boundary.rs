//! Keep persistence behind the domain storage modules, including SQL in macros.
use std::path::Path;
use syn::visit::Visit;

fn test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute
            .path()
            .segments
            .last()
            .is_some_and(|p| p.ident == "test")
            || (attribute.path().is_ident("cfg")
                && attribute
                    .parse_args::<syn::Path>()
                    .is_ok_and(|p| p.is_ident("test")))
    })
}

#[derive(Default)]
struct Boundary {
    violations: Vec<String>,
}

impl Boundary {
    fn string(&mut self, value: &str) {
        let value = value.trim_start();
        if [
            "SELECT ",
            "INSERT ",
            "UPDATE ",
            "DELETE ",
            "CREATE TABLE",
            "ALTER TABLE",
            "PRAGMA ",
        ]
        .iter()
        .any(|prefix| value.starts_with(prefix))
        {
            self.violations.push("SQL outside storage".into());
        }
    }

    fn tokens(&mut self, stream: proc_macro2::TokenStream) {
        for token in stream {
            match token {
                proc_macro2::TokenTree::Group(group) => self.tokens(group.stream()),
                proc_macro2::TokenTree::Literal(literal) => {
                    if let Ok(value) = syn::parse_str::<syn::LitStr>(&literal.to_string()) {
                        self.string(&value.value());
                    }
                }
                _ => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for Boundary {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if !test_only(&node.attrs) {
            syn::visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if !test_only(&node.attrs) {
            syn::visit::visit_item_fn(self, node);
        }
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let method = node.method.to_string();
        let sql = [
            "query_row",
            "query_map",
            "prepare_cached",
            "execute_batch",
            "pragma_update",
            "transaction",
            "transaction_with_behavior",
        ]
        .contains(&method.as_str());
        let dispatch = ["read", "write"].contains(&method.as_str())
            && node.args.len() == 2
            && matches!(
                node.args.first(),
                Some(syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(_),
                    ..
                }))
            );
        if sql || dispatch {
            self.violations
                .push(format!("storage method {method} outside storage"));
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_lit_str(&mut self, node: &'ast syn::LitStr) {
        self.string(&node.value());
    }
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.tokens(node.tokens.clone());
    }
}

fn inspect(path: &Path, failures: &mut Vec<String>) {
    let name = path.file_name().unwrap().to_string_lossy();
    if name == "storage"
        || name == "storage.rs"
        || name == "tests"
        || name == "tests.rs"
        || name.ends_with("_tests.rs")
    {
        return;
    }
    if path.is_dir() {
        for entry in std::fs::read_dir(path).unwrap() {
            inspect(&entry.unwrap().path(), failures);
        }
    } else if path.extension().is_some_and(|ext| ext == "rs") {
        let source = std::fs::read_to_string(path).unwrap();
        let syntax = syn::parse_file(&source).unwrap();
        let mut boundary = Boundary::default();
        boundary.visit_file(&syntax);
        for violation in boundary.violations {
            failures.push(format!("{}: {violation}", path.display()));
        }
    }
}

#[test]
fn handlers_and_workflows_use_domain_storage_operations() {
    let server = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut failures = Vec::new();
    inspect(&server.join("src"), &mut failures);
    for name in ["auth", "catalog", "jobs"] {
        inspect(
            &server.join(format!("../../crates/{name}/src")),
            &mut failures,
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
