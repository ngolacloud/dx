//! Geração de projectos (`dx new`/`dx init`). Produz um `Plan` de ficheiros;
//! a aplicação é transaccional (ver dx-manifest::txn).

use camino::Utf8Path;
use dx_core::{Change, Plan};

/// Normaliza o nome do crate: Cargo exige nomes sem separadores exóticos.
/// Aqui só substituímos `-` por `_` para o identificador interno quando
/// necessário; o nome do package mantém o original.
pub fn crate_base_name(name: &str) -> &str {
    // Usamos o último componente de caminho como nome do package.
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// Template de projecto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    Lib,
    Bin,
    /// Plataforma Seiva com composição gerida pelo DX (âncoras dx:seiva:*).
    Seiva,
}

pub fn plan_scaffold_template(dir: &Utf8Path, name: &str, template: Template) -> Plan {
    if template == Template::Seiva {
        return plan_seiva(dir, name);
    }
    plan_scaffold(dir, name, template == Template::Lib)
}

/// Scaffold de plataforma Seiva: Cargo.toml com metadata de composição +
/// `src/composition.rs` com âncoras geridas pelo DX + `src/main.rs`.
fn plan_seiva(dir: &Utf8Path, name: &str) -> Plan {
    let mut plan = Plan::new();
    let cargo_toml = format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         \n\
         [dependencies]\n\
         \n\
         # Marca de composição gerida: o adaptador Seiva do DX edita este\n\
         # ficheiro por âncoras (`dx add seiva-console-admin`).\n\
         [package.metadata.dx.seiva]\n\
         composition = \"src/composition.rs\"\n"
    );
    plan.push(Change::CreateFile {
        path: dir.join("Cargo.toml").to_string(),
        contents: cargo_toml,
    });
    plan.push(Change::CreateFile {
        path: dir.join("src/composition.rs").to_string(),
        contents: "//! Composição da plataforma Seiva — GERIDA pelo DX.\n\
                   //! Não remover os marcadores `// dx:seiva:*`.\n\
                   \n\
                   pub fn build_app() {\n\
                   \x20   // dx:seiva:use:begin\n\
                   \x20   // dx:seiva:use:end\n\
                   \n\
                   \x20   // dx:seiva:register:begin\n\
                   \x20   // dx:seiva:register:end\n\
                   }\n"
        .to_string(),
    });
    plan.push(Change::CreateFile {
        path: dir.join("src/main.rs").to_string(),
        contents: "mod composition;\n\nfn main() {\n\
                   \x20   composition::build_app();\n\
                   \x20   println!(\"plataforma Seiva (composição gerida pelo DX)\");\n\
                   }\n"
        .to_string(),
    });
    plan.push(Change::CreateFile {
        path: dir.join(".gitignore").to_string(),
        contents: "/target\n".to_string(),
    });
    plan
}

pub fn plan_scaffold(dir: &Utf8Path, name: &str, is_lib: bool) -> Plan {
    let mut plan = Plan::new();
    let cargo_toml = format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         \n\
         [dependencies]\n"
    );
    plan.push(Change::CreateFile {
        path: dir.join("Cargo.toml").to_string(),
        contents: cargo_toml,
    });

    if is_lib {
        plan.push(Change::CreateFile {
            path: dir.join("src/lib.rs").to_string(),
            contents: "//! Biblioteca gerada pelo DX.\n\npub fn add(a: u64, b: u64) -> u64 {\n    a + b\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn it_works() {\n        assert_eq!(add(2, 2), 4);\n    }\n}\n".to_string(),
        });
    } else {
        plan.push(Change::CreateFile {
            path: dir.join("src/main.rs").to_string(),
            contents: "fn main() {\n    println!(\"Olá do DX!\");\n}\n".to_string(),
        });
    }

    plan.push(Change::CreateFile {
        path: dir.join(".gitignore").to_string(),
        contents: "/target\n".to_string(),
    });

    plan
}
