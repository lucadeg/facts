# facts

Rust CLI that manages `.facts` files — flat lists of atomic, validatable truth statements about a project. Serves as both specification and documentation.

Read `.facts` in the project root for the full format spec, CLI reference, and architecture.

## Commands

```sh
cargo build              # build
cargo test               # run all tests (integration + inline unit)
cargo clippy             # lint
cargo fmt --check        # check formatting
```

Single test:

```sh
cargo test test_name -- --nocapture
```

## Code style

Minimal Rust. Two dependencies: `clap` (CLI) and `anyhow` (errors). No async, no FFI, single-threaded.

```rust
fn cleanup_empty_sections(sections: &mut Vec<Section>, path: &[usize]) {
    for depth in (0..path.len()).rev() {
        let current_path = &path[..=depth];
        let section = locate::navigate_to_section(sections, current_path);
        if section.facts.is_empty() && section.children.is_empty() {
            if depth == 0 {
                sections.remove(current_path[0]);
            } else {
                let parent_path = &current_path[..current_path.len() - 1];
                let parent = locate::navigate_to_section_mut(sections, parent_path);
                parent.children.remove(current_path[depth]);
            }
        } else {
            break;
        }
    }
}
```

## Testing

All user-facing features are tested end-to-end in `tests/cli.rs`. Each test gets an isolated temp directory with a `.git` marker. Unit tests are inline in their modules.

## Releasing

1. Bump version in `Cargo.toml`
2. Commit and push
3. `git tag vX.Y.Z && git push origin vX.Y.Z`
4. GitHub Actions builds binaries for 5 targets and creates the release

<!-- facts:start -->
## Fact-driven development

This project uses [facts](https://github.com/av/facts) for specification and documentation. The skill `facts skills show facts` has the full format spec and command reference.

### Mandatory fact-driven development workflow

Every feature, fix, or refactor follows this sequence. Do not skip or reorder steps.

1. **Read the spec.** `facts list` — orient yourself. Use filters to focus: `--section "cli/init"`, `--tags "draft"`, `--file api.facts`, `--manual`. Read the `## domain` section first if it exists — it defines the project vocabulary.
2. **Check coverage.** For every behavior you are about to implement, check whether a `@spec` fact already exists for it. Run `facts list --tags "spec"` and compare against your planned work.
3. **Write missing facts.** If any planned behavior has no corresponding fact, add it now: `facts add "what should be true when done" --section "relevant/section" --tags "spec"`. Do not proceed to step 4 until every behavior you plan to implement has a fact.
4. **Implement.** Write the code that makes the `@spec` facts true.
5. **Verify.** `facts check --tags "<tag>"` or `facts get <id>` — confirm each fact passes. Never run bare `facts check` unless asked. For manual facts (`?` in output): read what the fact claims, check the code, report PASS or FAIL with a one-line reason per fact.
6. **Tag done.** `facts edit <id> --remove-tag "spec" --add-tag "implemented"` for each verified fact.

**Lifecycle:** `@draft` → `@spec` → `@implemented`

**Skills** (invoke via `facts skills show <name>`):
- `facts-refine` — sharpen `@draft` facts into `@spec` with the user
- `facts-discover` — scan the codebase and sync facts to reality (only when explicitly asked)
- `facts-implement` — implement `@spec` facts in code, verify, tag `@implemented`
<!-- facts:end -->
