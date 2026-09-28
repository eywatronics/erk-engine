# Erk Engine — P0 Doğrulama ve Denetim Stratejisi

- **Tarih:** 2026-09-25
- **İlgili tasarım:** [p0-architecture.md](p0-architecture.md)
- **Amaç:** Tasarım dokümanındaki kuralların kod tarafında gerçekten uygulandığını
  otomatik olarak kanıtlamak. Bir kural CI'da zorlanmıyorsa, o kural yoktur.

---

## 1. Muhafız takvimi

Muhafız koruduğu şeyle aynı PR'da gelir (bkz. `CLAUDE.md`). Aşağıdaki tablo her
kuralın **ne zaman** ve **nasıl** zorlandığını, ve hangi kasıtlı ihlalle
denendiğini söyler. "Denendi" sütunu, ilgili planın Yürütme Notları'nda sonuç
yazıldığında doldurulur.

| Kural | Zorlama | Kasıtlı ihlal | Taş | Denendi |
|---|---|---|---|---|
| `unsafe` yasak | `[workspace.lints.rust] unsafe_code = "forbid"`; her crate `[lints] workspace = true` | `erk-dom` içinde bir `unsafe {}` bloğu → derleme hatası | M0 T1 | 2026-09-25, yakaladı |
| Her crate workspace lint'ini devralır | CI `guards` job'ı: `lints.workspace = true` içermeyen `Cargo.toml` (istisna listesi dışında) → hata | Bir crate'ten `[lints]` bloğunu silmek | M0 T1 | 2026-09-25, yakaladı |
| `erk-dom`'da `Rc` yok | `crates/erk-dom/clippy.toml` → `disallowed-types`; clippy `-D warnings` | `use std::rc::Rc;` ve bir alan → clippy hatası | M0 T2 | 2026-09-25, yakaladı |
| `Rc` yasağı susturulamaz | CI `guards`: `erk-dom` içinde `disallowed_types` geçmez (bir `allow` özniteliği lint'i kapatırdı) | `#[allow(clippy::disallowed_types)]` eklemek | M0 T2 | 2026-09-25, yakaladı |
| `erk-dom` yapraktır | CI `guards`: `cargo tree -p erk-dom --prefix none -e normal` çıktısında başka `erk-*` yok | `erk-dom`'a `erk-network` bağımlılığı eklemek | M0 T2 | 2026-09-25, yakaladı |
| html5ever + Stylo tek atom sürümü | CI `guards`: `cargo tree -d` çıktısında `web_atoms` veya `string_cache` iki sürümle görünürse hata | `html5ever`'ı 0.40.1'e çekmek (derleme de E0053 ile kırılıyor) | M0 T3 | 2026-09-25, yakaladı |
| Lint istisnası `unsafe`'i yine reddeder | CI `guards`: istisna listesindeki crate'ler (`erk-style`) `[lints.rust] unsafe_code = "deny"` yazmak zorunda | `erk-style`'da `deny` → `allow` | M0 T3 | 2026-09-25, yakaladı |
| `erk-style`'ın `unsafe` yüzeyi tam beş imza | CI `guards`: `allow(unsafe_code)` sayısı 5, `unsafe {` sayısı 0 | Altıncı bir `#[allow(unsafe_code)]` | M0 T3 | 2026-09-25, yakaladı |
| Stylo tutamağı tek işaretçi genişliğinde | `const _: () = assert!(size_of::<ErkNode>() == size_of::<usize>())` (derleme zamanı) | — (16 baytlık ilk tutamak Stylo'nun çalışma zamanı `assert`'ünde düştü; bu kontrol onu derlemeye taşıdı) | M0 T3 | — |
| Kabuk DOM'a dokunamaz | CI `guards`: `cargo tree -p erk-shell -e normal --depth 1` çıktısında `erk-dom` ve `erk-style` yok. `--depth 1` bilerek: `erk-shell → erk-renderer → erk-dom` zinciri dolaylı olarak her zaman görünür | `erk-shell`'e `erk-dom` bağımlılığı eklemek | M0 T7 | 2026-09-28, yakaladı |
| Kabuğun ekran görüntüsü renderer'ınkiyle aynı | `erk-shell/tests/screenshot.rs`: gerçek `erk --screenshot` çıktısı altın görüntüyle piksel piksel aynı | — | M0 T7 | — |
| Render çıktısı değişmez | Altın PNG testi (`cargo test`), çözülmüş piksellerle | Glif hinting'ini kapatmak | M0 T6 | 2026-09-25, yakaladı |
| Chrome'a yakınlık gerilemez | `tests/chrome_reference.rs`: sayfa başına içerik skoru `expectations.txt`'teki değere iki ondalıkta eşit olmalı, iki yönde de kırılır | UA'da body margin 8px → 10px; `blocks`'ta bir kutu 1px geniş (%99.97); `merhaba`'da beyaz kutu kaldırılınca (%10.37); sayfa `.htm` uzantısıyla | M0 T6b | 2026-09-25, yakaladı |
| Beklenti gerekçesiz düşmez | CI `guards`: `.github/scripts/check-reference-expectations.sh`, PR tabanıyla karşılaştırır; düşen satırda `# lowered:` yoksa ya da Chrome görüntüsü dururken beklenti silinmişse hata | Yorumsuz düşürme; beklenti satırını silme | M0 T6b | 2026-09-25, yakaladı |
| Boyama sırası: tüm arka planlar, sonra metin | `tests/paint.rs` + `paint-order` referans sayfası | Metni arka planlardan önce koymak | M0 T6 | 2026-09-25, yakaladı |
| Tuval beyaz üstüne harmanlanır, kare opak | `tests/paint.rs` + `canvas-alpha` referans sayfası | Beyaz tabanı kaldırmak | M0 T6 | 2026-09-25, yakaladı |
| Kutusuz kök/body tuvale renk yaymaz | `tests/paint.rs` | Kutu kontrolünü kaldırmak | M0 T6 | 2026-09-25, yakaladı |
| Lisans izin listesi | `cargo deny check licenses` | GPL lisanslı bir geliştirme bağımlılığı | M1 | — |
| WPT gerilemesi yok | wptrunner "erk" ürünü + beklenti dosyaları; taban çizgisinin altı PR'ı kırar | Geçen bir reftest'i bozan değişiklik | M1 | — |
| Renderer ağa bağımlı değil | CI: `cargo tree -p erk-renderer` çıktısında `erk-network`, `reqwest`, `hyper`, `tokio` yok | `erk-renderer`'a `reqwest` eklemek | M2 | — |
| OpenSSL/native-tls yok | `cargo deny check bans` → `openssl-sys`, `native-tls` | `native-tls` özelliği açık bir reqwest | M2 | — |
| Dış crate sınırları | `cargo deny` `[bans] deny = [{ crate = "...", wrappers = [...] }]` (ör. `ipc-channel` yalnızca `erk-ipc`) | `erk-renderer`'dan doğrudan `ipc-channel` | M3 | — |
| Workspace içi yön | `cargo xtask arch-check` (`cargo metadata` grafiği) — cargo-deny'nin workspace üyelerine uygulanması belgelenmemiş olduğu için | Yasak bir iç bağımlılık | M3 | — |
| Renderer dosya ve soket açamaz | Kum havuzu içinde renderer'ın dosya açma ve soket bağlama denemesi başarısız olmalı; zorunlu check | Sandbox'ı devre dışı bırakan bir bayrak | M3 | — |

`cargo tree` tabanlı kontroller M3'te `xtask arch-check`'e taşınır; o zamana kadar
tek satırlık CI adımlarıdır.

---

## 2. Sürüm sabitleme

- `Cargo.lock` depoya işlenir: Erk bir uygulama, kütüphane değil.
- `html5ever = "=0.39.0"` tam sürüme sabit. Stylo ile birlikte, ayrı bir PR'da
  yükseltilir; PR `cargo tree -d` çıktısını gösterir.
- Stylo'nun her 0.x sürümü kırıcı sayılır. Otomatik bağımlılık güncellemesi
  (Dependabot/Renovate) Stylo, html5ever, Taffy, Parley ve Vello ailesi için
  kapalıdır; bunlar elle yükseltilir.
- `rust-toolchain.toml`'daki sürüm `rustc --version` çıktısından alınır,
  belgeden değil.

---

## 3. Render doğrulaması

### 3.1 Altın PNG (M0)

`crates/erk-renderer/tests/golden.rs`, sayfayı `erk_renderer::render_html`
ile çizer ve çözülmüş pikselleri `crates/erk-renderer/tests/golden/` altındaki
referansla karşılaştırır. Kabuğun `--screenshot` yolu (Task 7) aynı işlevi
çağıracak; o yolun kendi testi Task 7'de gelir. Referans görüntü, onu
değiştiren değişiklikle **aynı commit'te** güncellenir ve gövde neden
değiştiğini söyler; ayrı commit, değişikliği yapan commit'i kırmızı bırakırdı.

Belirleyicilik için: sabit boyut (800×600), 1x DPI, depoda gömülü yazı tipi
(sistem yazı tipi yüklenmez), `vello_cpu` tek iş parçacığında **ve SIMD'siz
skaler yolda (`Level::fallback()`)**. `Level::baseline()` yetmiyor: x86_64'te
skaler, aarch64'te NEON. Kalan bir risk: skaler yol da platform `libm`'ine
giden işlevler kullanıyorsa mimariler arasında ufak farklar olabilir; macOS
arm64 CI'a girdiğinde (M2) altın görüntüler orada ayrıca doğrulanır.

### 3.2 Chrome referans testi (M0 T6b'den itibaren)

Aynı sayfa Chrome'da ve Erk'te çizilir; Chrome görüntüleri bir kez yakalanıp
depoya konur (`crates/erk-renderer/tests/reference/chrome/`). Skor, içerik
piksellerinin (tuval renginden **herhangi bir** farkı olan pikseller) kaçının
Chrome'la kanal başına 12'ye kadar farkla eşleştiği. Yalnızca tüm piksellere
bakılsaydı metin hiç çizilmeyen bir sayfa bile %95'in üstünde çıkardı.

Tolerans, referans sayfalarındaki en küçük düz renk farkının (17) altında
kalmak zorunda: 24'te `merhaba`'daki beyaz kutunun hiç çizilmemesi (fark 21)
fark edilmiyordu. Skor beklentiye iki ondalıkta tam eşit olmalı, test iki
yönde de kırılır; beklenti düşürmek `# lowered:` gerekçesi ister ve bunu CI
denetler. Kurallar proje kurallarında.

WPT reftest'leri (aşağıda) bununla çakışmaz: WPT, spesifikasyonun istediğini
iki sayfanın aynı çizilmesiyle doğrular; Chrome testi, gerçek bir tarayıcıdan
ne kadar uzak olduğumuzu ölçer.

### 3.3 WPT (M1'den itibaren)

- wptrunner'a "erk" ürünü: her reftest için `erk --screenshot` iki kez
  (test ve referans) çalışır, PNG'ler karşılaştırılır.
- Beklenti dosyaları (`.ini`, Servo'nun metadata yaklaşımı) bugünkü sonucu
  kaydeder. Kapı "her test geçmeli" değil, "beklentinin altına düşme"dir.
- Taban çizgisi JSON'u DioxusLabs `browser-wpt-results` biçiminde yayımlanır.
- Başlangıç dizinleri: `css/CSS2/normal-flow`, `css/CSS2/floats`,
  `css/css-display`, `css/css-flexbox`.

---

## 4. CI yapılandırması

M0 Task 1'den itibaren:

- `rust-checks` job'ı, `windows-latest` ve `ubuntu-latest` üzerinde:
  `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo build --workspace`, `cargo test --workspace`. Toolchain
  `rust-toolchain.toml`'dan `rustup toolchain install` ile kurulur.
- `guards` job'ı (ubuntu): mimari muhafızlar. İşletim sistemine bağlı olmadıkları
  için tek platformda koşar.
- Zorunlu check'ler: `rust-checks (ubuntu-latest)`, `rust-checks
  (windows-latest)`, `guards`.
- `macos-latest` M0'da yok; pencere katmanı macOS'ta ayrıca doğrulanacağı zaman
  (M2) eklenir.
- Stylo geldiğinde (M0 Task 3) CI'da Python 3 kurulu olmalı; GitHub'ın hazır
  imajlarında var, yine de sürüm adımda yazdırılır.

---

## 5. Kilometre taşı kabul kriterleri

Her taşın kabulü [roadmap.md](../plans/roadmap.md)'de. Kabul, bir komut ya da
test çıktısıyla gösterilebilir olmalı; "çalışıyor gibi" kabul değildir.
