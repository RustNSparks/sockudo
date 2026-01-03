# Contributing to Sockudo 🚀

Welcome! 🎉 We're thrilled that you'd like to contribute to **Sockudo**, a high-performance WebSocket server built in Rust. Your support helps improve the real-time ecosystem and benefits the community.

---

## 🧠 Before You Start

- **Read the [README](./README.md)** for project setup and usage.
- **Check existing issues and discussions** before opening a new one.
- Contributions of *all kinds* are welcome — code, docs, tests, issues, etc.

---

## 🛠️ Getting Started

### 1. Clone the Repository

```bash
git clone https://github.com/sockudo/sockudo
cd sockudo
```

### 2. Set Up Your Environment

* Copy and edit the example `.env` file:

  ```bash
  cp .env.example .env
  ```

* Run locally (Docker preferred):

  ```bash
  make quick-start
  ```

* Or using Cargo:

  ```bash
  cargo build
  cargo run
  ```

---

## 🤝 How to Contribute

### ✅ Good First Tasks

* Fix typos or improve documentation
* Tidy up logs or improve CLI feedback
* Add test coverage to critical modules
* Enhance monitoring or error handling

### 💡 Feature Contributions

1. Open a discussion or issue before implementing large changes.
2. Write clean, idiomatic Rust code.
3. Add tests for new features and ensure existing tests pass.

---

## 🎯 Code Style & Tools

* Run `cargo fmt` before committing.
* Use `cargo clippy` to lint your code.
* Follow Rust best practices and Sockudo’s architecture (channels, adapters, services).
* Ensure your feature fits into the modular structure: `core`, `adapters`, `config`, etc.

---

## 🧪 Running Tests

```bash
cargo test
```

To run integration tests (requires Redis, etc.):

```bash
make test
```

---

## 🚀 Submitting a Pull Request

1. Fork this repo and create your branch:

   ```bash
   git checkout -b feature/your-change
   ```

2. Make your changes with descriptive commit messages.

3. Push and open a PR to `main`:

   ```bash
   git push origin feature/your-change
   ```

4. Link related issues (e.g., `Closes #123`).

---

### 🔍 Code Review Process

All contributions go through a **code review**.
Please **do not merge your own pull requests**, even if all checks pass.
One of the maintainers will review your changes, suggest improvements if needed, and handle the merge. ✅

---

## 💬 Community & Conduct

* Be kind, inclusive, and respectful.
* All contributions are governed by our [Code of Conduct](./CODE_OF_CONDUCT.md).

* 💬 **Join us on [Discord](https://discord.gg/BSdkg2JR)** — we discuss new features, ideas, and help new contributors get onboarded!  
  If you're not sure where to start, just post in the `#contributions` channel and we'll figure something out together. 💡

---


## 📜 License

By contributing, you agree that your contributions will be licensed under the [AGPL-3.0](./LICENSE).

Thank you for making Sockudo better! 🦀💬
