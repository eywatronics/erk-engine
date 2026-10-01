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
| `erk-dom`'da referans sayımı yok | `crates/erk-dom/clippy.toml` → `disallowed-types` (`Rc`, `rc::Weak`, `Arc`, `sync::Weak`); clippy `-D warnings` | `use std::rc::Rc;` ve bir alan → clippy hatası; aynısı `Arc` ile (kanarya) | M0 T2, T8 | 2026-09-25 (`Rc`), 2026-09-28 (`Arc`), yakaladı |
| Referans sayımı yasağı susturulamaz | CI `guards` (`check-dom-rc-ban.sh`): `erk-dom`'da lint'i adıyla ya da onu içeren bir grupla (`clippy::style`, `clippy::all`, `warnings`) susturan öznitelik yok; ayrıca crate'e eklenen kanarya `Rc` ve `Arc` alanlarını clippy gerçekten reddetmeli | `#[allow(clippy::disallowed_types)]`; crate düzeyinde `#![allow(clippy::all)]`; `node.rs`'te `#![allow(clippy::style)]`; workspace tablosunda `disallowed_types = "allow"`; `clippy.toml`'u silmek | M0 T2, T8 | 2026-09-25 (ilki); 2026-09-28 (diğer dördü, eski muhafız hepsini geçiriyordu), yakaladı |
| `erk-dom` yapraktır | CI `guards`: `cargo tree -p erk-dom -e normal --target all --all-features --locked --prefix none` çıktısında başka `erk-*` yok | `erk-dom`'a `erk-network` bağımlılığı; yalnızca Linux için tanımlı bir `erk-network` bağımlılığı (eski komut Windows'ta görmüyordu) | M0 T2, T8 | 2026-09-25, 2026-09-28, yakaladı |
| html5ever + Stylo tek atom sürümü | CI `guards`: `cargo tree -d --target all --all-features` çıktısında `web_atoms` veya `string_cache` iki sürümle görünürse hata | `html5ever`'ı 0.40.1'e çekmek (derleme de E0053 ile kırılıyor) | M0 T3 | 2026-09-25, yakaladı |
| Lint istisnası `unsafe`'i yine reddeder | CI `guards`: istisna listesindeki crate'ler (`erk-style`) `[lints.rust] unsafe_code = "deny"` ve `unsafe_op_in_unsafe_fn = "forbid"` yazmak zorunda | `erk-style`'da `deny` → `allow`; `forbid` satırını silmek | M0 T3, T8 | 2026-09-25, 2026-09-28, yakaladı |
| `erk-style`'ın `unsafe` yüzeyi tam beş imza | CI `guards` (`check-style-unsafe-surface.sh`): crate'in tamamında (`src`, `tests`) `allow`/`expect(unsafe_code)` tam 5; yorum dışı `unsafe` belirteci tam 5, beşi de `unsafe fn`. Gövdelerde güvensiz işlem `forbid` ile derleme hatası | Altıncı `#[allow(unsafe_code)]`; `#[expect(unsafe_code)] unsafe impl Sync`; `#[allow(unsafe_code, reason = ..)]` ile bir blok; `tests/`'te izinli bir blok (eski sayım bu üçünü görmüyordu); bir `unsafe fn`'e `#[allow(unsafe_op_in_unsafe_fn)]` → E0453 | M0 T3, T8 | 2026-09-25 (ilki), 2026-09-28 (diğerleri), yakaladı |
| Stylo tutamağı tek işaretçi genişliğinde | `const _: () = assert!(size_of::<ErkNode>() == size_of::<usize>())` (derleme zamanı) | Tutamağa ikinci bir alan (`u32`) eklemek → E0080 (16 baytlık ilk tutamak Stylo'nun çalışma zamanı `assert`'ünde düşmüştü; bu kontrol onu derlemeye taşıdı) | M0 T3 | 2026-09-28, yakaladı |
| Kabuk DOM'a dokunamaz | CI `guards`: `cargo tree -p erk-shell -e normal --target all --all-features --locked --depth 1` çıktısında `erk-dom` ve `erk-style` yok. `--depth 1` bilerek: `erk-shell → erk-renderer → erk-dom` zinciri dolaylı olarak her zaman görünür | `erk-shell`'e `erk-dom` bağımlılığı; yalnızca Linux için tanımlı ya da isteğe bağlı bir özelliğin arkasındaki `erk-dom` (eski komut ikisini de görmüyordu) | M0 T7, T8 | 2026-09-28, yakaladı |
| Renderer'ın yüzeyi iş parçacığı ve düz veri mesajlar | CI `guards` (`check-renderer-surface.sh`): `lib.rs`'teki `pub` satırları gözden geçirilmiş listeye eşit; mesaj tiplerine `impl` yalnızca `messages.rs`'te (ve `lib.rs`'te `to_png`); `messages.rs` yol (`::`), `Arc`, `Mutex`, `Cell`, `Box`, `dyn`, ödünç referans içermez; `erk-shell`'de `render_html` yok | `pub use erk_dom;`; `Shared(Arc<Mutex<String>>)` mesajı; kabukta `render_html`; `paint.rs`'te `impl Deref for Frame` | M0 T8 | 2026-09-28, yakaladı |
| `cargo tree` hatası adımı düşürür | CI adımları ağacı önce bir değişkene okur; `if cargo tree … grep` biçiminde bir hata "eşleşme yok" sayılırdı | Kilitle uyuşmayan manifest → adım 101 ile düşer | M0 T8 | 2026-09-28, yakaladı |
| CI kilit dosyasıyla derler | clippy, build, test ve `cargo tree` adımlarında `--locked` | Kilitte olmayan bir bağımlılık eklemek → "cannot update the lock file" | M0 T8 | 2026-09-28, yakaladı |
| Commit mesajları ve PR yapay zekâ aracı adı taşımaz | CI `guards` (`check-commit-messages.sh`): PR'daki commit mesajları, PR başlığı ve açıklaması; tek izinli co-author satırı projeninki | Yabancı bir `Co-authored-by`; gövdede araç adı; PR açıklamasında "Generated with" | M0 T8 | 2026-09-28, yakaladı |
| Kabuğun ekran görüntüsü renderer'ınkiyle aynı | `erk-shell/tests/screenshot.rs`: gerçek `erk --screenshot` çıktısı altın görüntüyle piksel piksel aynı | Kabuğun ekran görüntüsü yüksekliği 600 → 601 | M0 T7 | 2026-09-28, yakaladı |
| Pencere modunda renderer çökmesi bildirilir | `window.rs`: `finish` birim testi; kanal kapanınca pencere kapanır, çıkış kodu 1 | `finish`'in renderer sonucunu yok sayması; renderer'a enjekte edilen panik → `erk: the renderer thread panicked`, çıkış 1 | M0 T8 | 2026-09-28, yakaladı |
| Renderer boyut bilinmeden çizmez, son boyut kazanır | `tests/thread.rs`, zaman aşımlı beklemeyle | Varsayılan bir boyut (eski test `Shutdown` kuyrukta beklediği için geçiyordu); yalnızca ilk boyutu tutmak (eski test askıda kalırdı) | M0 T7, T8 | 2026-09-28, yakaladı |
| Render çıktısı değişmez | Altın PNG testi (`cargo test`), çözülmüş piksellerle | Glif hinting'ini kapatmak | M0 T6 | 2026-09-25, yakaladı |
| Chrome'a yakınlık gerilemez | `tests/chrome_reference.rs`: sayfa başına içerik skoru `expectations.txt`'teki değere iki ondalıkta eşit olmalı, iki yönde de kırılır; aynı sayfa için ikinci satır ve yakalamadan sonra değişen sayfa (`chrome/pages.txt`) testi kırar | UA'da body margin 8px → 10px; `blocks`'ta bir kutu 1px geniş (%99.97); `merhaba`'da beyaz kutu kaldırılınca (%10.37); sayfa `.htm` uzantısıyla; `blocks 99.00` ikinci satırı; `blocks.html`'e bir yorum eklemek | M0 T6b, T8 | 2026-09-25, 2026-09-28, yakaladı |
| Kutular Chrome'la örtüşür | `tests/chrome_reference.rs`: her referans sayfasında kutusu olan her elemanın konumu ve boyutu, yakalama sırasında Chrome'dan ölçülen kutuyla 1 CSS pikseli içinde (`chrome/<sayfa>.geometry.txt`); satır içi elemanlar atlanır ve sayılır | `line-height: normal`'daki yuvarlamayı kaldırmak (`paragraphs`: 6 kutudan 5'i tutuyor) | M1.3 | 2026-09-30, yakaladı |
| Tolerans düz renkleri birbirinden ayırır | `the_tolerance_cannot_hide_a_missing_background`: her Chrome görüntüsünde 1000 pikselden fazlasını kaplayan renk çiftleri toleranstan fazla farklı olmalı | `TOLERANCE` 12 → 24 (`blocks`: 17) | M0 T8 | 2026-09-28, yakaladı |
| Beklenti gerekçesiz düşmez | CI `guards`: `.github/scripts/check-reference-expectations.sh`, PR tabanıyla karşılaştırır. Düşen satırda boş olmayan ve tabandaki satırda durmayan bir `# lowered:` gerekçesi olmalı; Chrome görüntüsü dururken beklenti silinemez; aynı sayfa için ikinci satır olamaz; yeniden adlandırılan görüntü eski adının skorunu taşır | Yorumsuz düşürme; beklenti satırını silme; `paragraphs`'ı eski gerekçeyle yeniden düşürmek; boş `# lowered:`; ikinci satır; `blocks`'u `blocks2` olarak düşük skorla yeniden adlandırmak (son dördünü eski betik geçiriyordu) | M0 T6b, T8 | 2026-09-25, 2026-09-28, yakaladı |
| Boyama sırası: tüm arka planlar, sonra metin | `tests/paint.rs` (display list sırası ve sarı zemindeki metin pikselleri, kırmızı kanalla) + `paint-order` referans sayfası | Metni arka planlardan önce koymak; glifleri ayrı bir geçişte önce boyamak (eski mavi kanal ölçütü geçiriyordu) | M0 T6, T8 | 2026-09-25, 2026-09-28, yakaladı |
| Tuval beyaz üstüne harmanlanır, kare opak | `tests/paint.rs` + `canvas-alpha` referans sayfası | Beyaz tabanı kaldırmak | M0 T6 | 2026-09-25, yakaladı |
| Kutusuz kök/body tuvale renk yaymaz | `tests/paint.rs` | Kutu kontrolünü kaldırmak | M0 T6 | 2026-09-25, yakaladı |
| Sunumsal öznitelikler stile girer | `erk-style/tests/computed.rs`: `bgcolor` → `background-color`, `align` → `text-align` (hizalama M1'de çizilir) | İki eşlemeyi ayrı ayrı kapatmak | M0 T8 | 2026-09-28, yakaladı |
| Çekirdekte dosya, ağ, süreç, ortam ve saat yok | CI `guards` (`check-core-io.sh`): `erk-dom`, `erk-style`, `erk-renderer` `src`'sinde `std::fs`, `std::net`, `std::process`, `std::env`, `File::`, `TcpStream`, `Command::new`, `Instant::now`, `SystemTime` ve benzerleri yok; kaynak, ortam ve zaman host'tan gelir (p1-embedded §2.4) | `std::fs::read_to_string`; `use std::{fs}` + `fs::read`; `Instant::now()` | M0.5 öncesi | 2026-09-30, yakaladı |
| `erk-renderer` pencere katmanını bilmez | CI `guards`: `cargo tree -p erk-renderer --target all --all-features` çıktısında `winit` ve `softbuffer` yok (p1-contract §11) | `erk-renderer`'a `winit` eklemek | M1.0 | 2026-09-30, yakaladı |
| Lisans izin listesi | `cargo deny check licenses` (MPL-2.0 ve OFL-1.1 dahil) | GPL lisanslı bir geliştirme bağımlılığı | M1 | — |
| WPT gerilemesi yok (CSS dizinleri) | wptrunner "erk" ürünü + beklenti dosyaları; taban çizgisinin altı PR'ı kırar | Geçen bir reftest'i bozan değişiklik | M1 | — |
| `render_html` hiçbir girdide paniklemez ya da çökmez | `tests/robustness.rs`: sabit tohumlu 300 bozuk belge, `tests/robustness/` korpusu, gerçek renderer iş parçacığında 5000 düzey iç içelik; ayrıştırıcı derinliği Chrome gibi sınırlar (`erk-dom`, `nesting_stops_where_chrome_stops`), renderer iş parçacığının yığını 16 MiB. cargo-fuzz job'ı ayrı PR'da | Derinlik sınırını kaldırmak; renderer yığınını varsayılana döndürmek (ikisi de yığın taşması); bir kontrol karakterinde panik | M1.3 | 2026-10-01, yakaladı |
| İkili boyutu bütçede, bütçe gerekçesiz yükselmez | CI `size` job'ı (`check-size-budget.sh`): Linux yayın ikilisi `.github/size-budget.txt`'teki tavanın altında; tavanı yükseltmek, taban commit'e göre yeni bir `# raised:` gerekçesi ister | Tavanın üstünde bir ikili (ilk gerçek Linux ölçümü geçici tavanı aştı); bütçesiz dosya; eksik ikili; gerekçesiz, boş gerekçeli ve eski gerekçeli yükseltme | M1.0 | 2026-09-30, yakaladı |
| CSS matrisindeki her "Supported" satırın testi var | CI `guards` (`check-css-support.sh`): her Supported satır bir test fonksiyonu ya da test dosyası adlandırır ve o test var | Var olmayan bir test adı; test adı olmayan satır; var olmayan test dosyası | M1.0 | 2026-09-30, yakaladı |
| `unsafe` yalnızca `erk-style` ve `erk-ffi`'de | Lint devralma istisna listesi; `erk-ffi` de `deny` + öğe başına izin | `erk`'e `unsafe` blok | M3 | — |
| `erk.h` güncel | CI: cbindgen ile yeniden üretilen başlık depodakiyle aynı | Başlığı güncellemeden C-ABI'yi değiştirmek | M3 | — |
| C örneği derlenir ve çalışır | CI: C örneği `erk-ffi`'ye bağlanıp bir sayfa açar | C-ABI'de uyumsuz bir imza | M3 | — |
| FFI'dan panik sızmaz, eski id ve yanlış iş parçacığı hata kodu döner | `erk-ffi` testleri | `catch_unwind`'i kaldırmak; iş parçacığı denetimini kaldırmak | M3 | — |
| `Mutation` dizileri motoru bozamaz | cargo-fuzz, eski `NodeId`'ler dahil | Nesil denetimini kaldırmak | M4 | — |
| `erk-invalidation` yalnızca `erk-dom`'a bağımlı | CI `guards`: `cargo tree -p erk-invalidation --target all --all-features --locked` derinlik 1'de projeden yalnızca `erk-dom` | `erk-invalidation`'a `erk-style` eklemek | M5 | — |
| Artımlı render tam yeniden hesapla aynı | Mutation fuzz'ı her diziyi artımlı ve tam yoldan geçirir, display list'ler eşit; kısmi kare ile tam kare piksel piksel aynı (altın test) | Bir parçanın hasarını üretmemek; erken kesmeyi çıktı değiştiğinde de uygulamak | M5 | — |
| Çekirdekte JS motoru yok | CI `guards`: `cargo tree --target all --all-features --locked` ile çekirdek crate'lerin, `erk`'in ve `erk-ffi`'nin ağacında bilinen JS motorları (`boa_engine`, `rquickjs`, `quickjs`, `v8`, `deno_core`) yok; `erk-script`'in projeden tek bağımlılığı `erk` | Çekirdeğe bir JS motoru eklemek; `erk-script`'i `erk-dom`'a bağlamak; motoru bir özelliğin arkasına saklamak | M6 | — |

İç bağımlılık yönü bugün `cargo tree` adımlarıyla denetleniyor; crate sayısı
artarsa (M3'te `erk`, `erk-ffi`) bir `xtask arch-check`'e taşınması
değerlendirilir. Ağ, kum havuzu ve süreç sınırı muhafızları, bu hedefler yol
haritasından çıktığı için (p1-embedded.md) takvimden çıkarıldı.

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
referansla karşılaştırır. Kabuğun `--screenshot` yolu aynı işlevi renderer
iş parçacığı üzerinden çağırır; `crates/erk-shell/tests/screenshot.rs` gerçek
`erk` ikilisinin çıktısını aynı altın görüntüyle piksel piksel karşılaştırır.
Referans görüntü, onu
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
- Başlangıç dizinleri (M1'in kapsamına göre): `css/CSS2/normal-flow`,
  `css/css-flexbox`, `css/css-position`, `css/css-text`. Float ve tablo
  dizinleri kapsam dışı (css-support.md "Not planned").

---

## 4. CI yapılandırması

M0 Task 1'den itibaren:

- `rust-checks` job'ı, `windows-latest` ve `ubuntu-latest` üzerinde:
  `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo build --workspace`, `cargo test --workspace`. Toolchain
  `rust-toolchain.toml`'dan `rustup toolchain install` ile kurulur.
- `guards` job'ı (ubuntu): mimari muhafızlar. Tek platformda koşar: statik
  kontroller işletim sistemine bağlı değil, `cargo tree` kontrolleri de
  `--target all --all-features` ile her platformun ve her özelliğin
  bağımlılıklarını çözer. Betikler `.github/scripts/` altında.
- Derleme, clippy, test ve `cargo tree` `--locked` ile: CI, `Cargo.lock`'taki
  sürümlerle derler. İki job'ın da zaman aşımı var (45 ve 20 dakika); bir
  kanalı sonsuza kadar bekleyen test çalıştırmayı altı saat tutmaz.
- Zorunlu check'ler: `rust-checks (ubuntu-latest)`, `rust-checks
  (windows-latest)`, `guards`.
- `macos-latest` M0'da yok; pencere katmanı macOS'ta ayrıca doğrulanacağı zaman
  (M2) eklenir.
- Stylo'nun derlemesi Python 3 ister. GitHub'ın hazır imajlarında var; CI
  sürümü ayrı bir adımda yazdırır (Windows'ta `python`, diğerlerinde
  `python3`, Stylo'nun arama sırası).

---

## 5. Kilometre taşı kabul kriterleri

Her taşın kabulü [roadmap.md](../plans/roadmap.md)'de. Kabul, bir komut ya da
test çıktısıyla gösterilebilir olmalı; "çalışıyor gibi" kabul değildir.
