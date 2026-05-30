pkgname=tycode
pkgver=0.0.0
pkgrel=1
pkgdesc="AI system agent with a TUI that works with any LLM provider"
arch=('x86_64')
url="https://github.com/AlphaGlider25/TyCode"
license=('GPL3')
depends=()
makedepends=('rust' 'cargo')
source=("${pkgname}-${pkgver}.tar.gz::https://github.com/AlphaGlider25/TyCode/archive/v${pkgver}.tar.gz")
sha256sums=('SKIP')

build() {
  cd "${pkgname}-${pkgver}"
  cargo build --release
}

package() {
  cd "${pkgname}-${pkgver}"
  install -Dm755 target/release/tycode "${pkgdir}/usr/bin/tycode"
  install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
  install -Dm644 README.md "${pkgdir}/usr/share/doc/${pkgname}/README.md"
}
