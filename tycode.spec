Name:           tycode
Version:        0.0.0
Release:        1%{?dist}
Summary:        AI system agent with a TUI that works with any LLM provider

License:        GPL-3.0
URL:            https://github.com/AlphaGlider25/TyCode
Source0:        https://github.com/AlphaGlider25/TyCode/archive/v%{version}.tar.gz

BuildRequires:  rust cargo

%description
TyCode is an AI system agent with a terminal user interface (TUI) that works
with any LLM provider including Anthropic, OpenAI, Ollama, and Gemini.

%prep
%autosetup -n TyCode-%{version}

%build
cargo build --release

%install
install -Dm755 target/release/tycode %{buildroot}%{_bindir}/tycode
install -Dm644 LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE
install -Dm644 README.md %{buildroot}%{_docdir}/%{name}/README.md

%files
%{_bindir}/tycode
%license LICENSE
%doc README.md

%changelog
* Fri May 09 2026 TyCode <noreply@example.com> - 0.0.0-1
- Initial release
