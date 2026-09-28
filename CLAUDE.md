# Erk Engine — proje kuralları

Bu dosya depoda tutulur ve `.gitignore`'a **eklenmez**: kurallar makineye değil
projeye aittir.

## Commit kuralları

- Commit mesajlarının sonuna **yalnızca** şu satır eklenir:

  ```
  Co-authored-by: ismet kabatepe <>
  ```

- Commit mesajlarında, açıklamalarında veya trailer'larında **hiçbir yapay zekâ
  aracının adı geçmez.** `Co-Authored-By: Claude`, `Generated with ...` ve
  benzeri satırlar eklenmez. Bu dosyanın adı da commit mesajına yazılmaz;
  "proje kuralları" denir. CI (`guards` job'ı) PR'daki commit mesajlarını, PR
  başlığını ve açıklamasını bu iki kural için denetler
  (`.github/scripts/check-commit-messages.sh`). Squash birleştirmede GitHub'ın
  oluşturduğu mesaj PR'dan sonra yazıldığı için denetlenemez: birleştirmeden
  önce elle gözden geçirilir.
- Mesaj gövdesi *ne* yapıldığını değil, **neden** yapıldığını anlatır. Kararın
  gerekçesi ve reddedilen alternatif, koda bakılarak anlaşılamayacak tek şeydir.
- Konu satırı 72 karakteri geçmez; gövde satırları 72 karakterde sarılır.
- Conventional Commits öneki kullanılır: `feat`, `fix`, `chore`, `docs`,
  `test`, `refactor`, `perf`, `ci`. Kapsam genellikle crate ya da alt sistemdir:
  `feat(dom): ...`, `fix(layout): ...`.
- Commit mesajları İngilizce yazılır.

## Dal ve PR akışı

- `main` korumalıdır; doğrudan push yapılmaz.
- Her iş kendi dalında yapılır, PR ile birleştirilir. Dal önekleri: `feat/`,
  `fix/`, `refactor/`, `docs/`, `chore/`, ve kilometre taşı işleri için `m0/`,
  `m1/`, ...
- PR açıklamalarında da yapay zekâ aracı adı veya imzası bulunmaz.
- **Yarım kalmış bir kilometre taşının üzerine bir sonraki başlamaz.**

## Dil

| Türkçe | İngilizce |
|---|---|
| `CLAUDE.md`, `docs/design/`, `docs/plans/` | `README.md`, `ARCHITECTURE.md`, `CONTRIBUTING.md`, kod, tanımlayıcılar, yorumlar, commit mesajları, PR'lar |

Açık kaynak bir motorun katkıcıları koda ve geçmişe İngilizce bakar; tasarım
tartışması ise bu projede Türkçe yürüyor.

## Muhafız ilkesi

İki cümle, ikisi de bağlayıcı:

1. **Bir kural CI'da zorlanmıyorsa, o kural yoktur.** Yorumda ya da dokümanda
   kalan bir mimari kural ilk aceleci PR'da delinir, ve delindiği anda üzerine
   kod yazılır.
2. **Muhafız, koruduğu şeyle aynı PR'da gelir.** Önceden gelmez: henüz var
   olmayan bir süreç sınırını koruyan kural, prototiplemeyi yavaşlatır ve
   sınırın nerede olacağını bilmeden çizer. Sonraya da kalmaz: sınırın
   kendisiyle birlikte gelmeyen muhafız hiç gelmez.

Her yeni muhafız **kasıtlı bir ihlalle** denenir; yakalandığı ve geri alınınca
geçtiği ilgili planın "Yürütme Notları"na yazılır. Tek bir ihlal yetmez: aynı
kuralı delen **eşdeğer** yollar da düşünülür (başka bir yazım, başka bir
dizin, başka bir platform, bir grup izni). M0 kabul denetimi, her biri kendi
ihlalini yakalayan muhafızlardan beşinin eşdeğer bir ihlali geçirdiğini
buldu.

Bir muhafız betiği grep'in hatasını "eşleşme yok" saymaz: yok bir yol ya da
başarısız bir `cargo tree`, adımı geçirmez, düşürür.

## Bugün geçerli mimari kurallar

| Kural | Neden | Zorlama | Devreye girdiği yer |
|---|---|---|---|
| `unsafe` yasak | Bellek güvenliği projenin var olma sebebi. İstisna yalnızca adıyla listelenmiş crate'lerde | `[workspace.lints.rust] unsafe_code = "forbid"`, her crate `lints.workspace = true`; istisnalar `unsafe_code = "deny"` ve `unsafe_op_in_unsafe_fn = "forbid"` yazmak zorunda | M0 Task 1 |
| `erk-style`'ın `unsafe` yüzeyi tam beş imza | Stylo'nun `TElement`'i beş metodu `unsafe fn` tanımlıyor; bunları uygulamak gövde güvenli olsa bile lint ihlali. Başka hiçbir `unsafe` kod yok | CI (`check-style-unsafe-surface.sh`): crate'in tamamında (src, tests) yorum dışı `unsafe` belirteci tam 5 ve beşi de `unsafe fn`; `allow`/`expect(unsafe_code)` tam 5. Gövdede güvensiz işlem `forbid` ile hata | M0 Task 3 |
| `erk-dom` içinde referans sayımı yok | Döngüsel ağaçta referans sayımı sızıntı üretir; DOM arena + `NodeId` ile çalışır | `crates/erk-dom/clippy.toml` → `disallowed-types` (`Rc`, `rc::Weak`, `Arc`, `sync::Weak`), clippy `-D warnings`; `check-dom-rc-ban.sh`: lint'i ya da onu içeren grupları susturan öznitelik yok, ve crate'e eklenen bir kanarya tipini clippy'nin gerçekten reddettiği doğrulanır (silinen `clippy.toml`'u, workspace tablosundaki `allow`'u yakalar) | M0 Task 2 |
| `erk-dom` projeden hiçbir şey import etmez | En alttaki katman; parser dışında her şey ona bağlanır, o hiçbir şeye | CI'da `cargo tree -p erk-dom --target all --all-features` kontrolü (dev-dependency'ler bilerek dışarıda: yalnızca testlere girer) | M0 Task 2 |
| Kabuk ile renderer yalnızca mesajla konuşur | M3'te renderer ayrı sürece taşındığında değişen tek şey taşıma katmanı olsun. Paylaşılan değiştirilebilir durum (`Arc<Mutex<Dom>>`) süreç ayrımını yeniden yazıma çevirir | `erk-shell` doğrudan `erk-dom`'a ya da `erk-style`'a bağımlı olamaz (`cargo tree --depth 1 --target all --all-features`). `check-renderer-surface.sh`: `erk-renderer`'ın dış yüzeyi gözden geçirilmiş listeye eşit; mesajlar `messages.rs`'te ve o dosya yalnızca prelude tiplerini kullanır (yol yok, `Arc`/`Mutex`/`Cell`/`Box`/ödünç yok); kabuk `render_html` çağırmaz. `'static` olmayan referansı zaten `spawn`'ın imzası dışlar | M0 Task 7, T8 |
| `html5ever` ve `stylo` birlikte yükseltilir | İkisi `web_atoms`/`string_cache` üzerinden aynı atom tiplerini paylaşmak zorunda; html5ever 0.40 ile Stylo 0.21 uyumsuz | `html5ever = "=0.39.0"` sabit; CI'da `web_atoms` ve `string_cache` için tek sürüm kontrolü (tüm hedefler) | M0 Task 3 |
| CI kilit dosyasıyla derler | Altın görüntüler ve Chrome skorları `Cargo.lock`'taki sürümlerle üretildi; kilitten sapan bir manifest CI'da sessizce yeniden çözülmemeli | clippy, build, test ve `cargo tree` adımlarında `--locked` | M0 Task 8 |

## Kural takvimi

Aşağıdakiler bugün **yok**; korudukları şey geldiğinde gelirler. Tam liste ve
zorlama yöntemi: [p0-verification.md](docs/design/p0-verification.md).

| Taş | Gelen kural |
|---|---|
| M1 | Lisans izin listesi (`cargo deny check licenses`). İlk MPL-2.0 bağımlılıklar (Stylo, selectors) M0 Task 3'te girdi; liste `erk-renderer`'daki yazı tiplerinin OFL-1.1'ini de kapsamalı |
| M1 | WPT gerileme yasağı: taban çizgisinin altına düşen PR birleşmez |
| M2 | `erk-renderer` ağ crate'lerine bağımlı olamaz; ağ yalnızca `erk-network` arayüzünden |
| M2 | OpenSSL ve `native-tls` yasak (`cargo deny` bans) |
| M3 | Süreç sınırı: `cargo deny` `wrappers`, `xtask arch-check`, sandbox testi zorunlu check |
| M3 | `unsafe` istisnası: `erk-sandbox` (işletim sistemi API'leri) |
| M4 | `unsafe` istisnası: JS motoru bağlama crate'i (mozjs seçilirse) |

## unsafe ve C/C++ politikası

- `unsafe` yalnızca adıyla listelenmiş crate'lerde bulunur. Bugün liste tek
  kalem: **`erk-style`**, FFI değil ama Stylo'nun trait imzası yüzünden; beş
  `unsafe fn` imzası, gövdeleri güvenli. İstisna crate'i workspace lint'ini
  devralmaz, kendi `[lints]` tablosunda `unsafe_code = "deny"` ve
  `unsafe_op_in_unsafe_fn = "forbid"` yazar ve izni öğe bazında
  `#[allow(unsafe_code)]` ile, gerekçe yorumuyla verir.
- C/C++ bağımlılığı yalnızca iki yerde kabul edilir: JS motoru (M4, mozjs
  seçilirse) ve TLS kripto sağlayıcısı (M2, `aws-lc-rs`). Yeni bir C/C++
  bağımlılığı bir tasarım kararıdır ve `docs/design/` altına yazılır.
- Kural "saf Rust" değil, **"OpenSSL/native-tls yok"**: rustls'in protokolü
  Rust, kriptosu değil. Yanlış iddia etmektense doğru kural.

## Derleme önkoşulları

- Rust stable; sürüm `rust-toolchain.toml`'da sabit.
- Windows: MSVC Build Tools (C++ iş yükü).
- Python 3: Stylo'nun `build.rs`'i `properties/build.py`'yi çalıştırır. Önce
  `PYTHON3` ortam değişkenine, yoksa Windows'ta `python.exe`'ye bakar.
- LLVM/libclang **gerekmez**: Stylo'da bindgen yalnızca `gecko` özelliğinde.

## Gizlilik ve güvenlik

- **Telemetri yok.** Hiçbir kod, kullanıcının başlatmadığı bir ağ isteği
  atmaz.
- Loglara sayfa içeriği, çerez, form verisi veya kimlik bilgisi yazılmaz.
- Hedef mimaride renderer güvenilmezdir: renderer'ın iddia ettiği origin'e,
  kendi raporladığı güvenlik durumuna asla güvenilmez (M3'ten itibaren
  zorlanır, tasarım bugünden buna göre yapılır).

## Test disiplini

- Her değişiklik testle başlar.
- `cargo test --workspace` yeşil olmadan commit yapılmaz. **Derlenmemiş Rust
  kodu commit'lenmez.**
- Render çıktısı altın PNG veya reftest ile doğrulanır. "Gözle baktım, doğru"
  bir test değildir.
- **Bir test, koruduğu şey bozulunca kırılmalı.** Yeni ya da düzeltilen her
  test, korumak istediği hatayı bilerek üreten bir mutasyonla denenir. M0
  kabulünde boyama sırası testi, metin sarının altında kalsa da geçiyordu
  (sarının mavi kanalı zaten 0'dı).
- Bir kanal ya da iş parçacığı bekleyen test zaman aşımıyla bekler: askıda
  kalan test kırılmış test değildir.
- Planın bir varsayımı yürütmede yanlış çıkarsa, düzeltilmiş gerçek o planın
  "Yürütme Notları"na yazılır; sonraki görevler oradan okunur.

## Referans testi (Chrome karşılaştırması)

Aynı HTML hem Chrome'da hem Erk'te çizilir ve görüntüler piksel piksel
karşılaştırılır (`crates/erk-renderer/tests/chrome_reference.rs`). Altın test
Erk'in kendi çıktısıyla **tam eşitliği** korur; referans testi **Chrome'a
yakınlığı** korur. Piksel piksel aynılık hedef değil (kenar yumuşatma ve
hinting farklı); sayfa başına bir içerik skoru var, skor kayıtlı beklentiye
eşit kalır ve yalnızca yazılı gerekçeyle düşer. İçerik pikseli, tuval
renginden **herhangi bir** farkı olan piksel; eşleşme toleransı kanal başına
12 ve referans sayfalarındaki en küçük düz renk farkının (17) altında kalmak
zorunda. Bunu `the_tolerance_cannot_hide_a_missing_background` testi denetler:
her Chrome görüntüsünde 1000 pikselden fazlasını kaplayan renkler düz sayılır
ve ikisi arasındaki fark toleransın üstünde olmalı.

- **Render'ı etkileyen her önemli değişiklikten sonra çalıştırılır:** stil,
  layout, metin, boyama, UA stil sayfası, render bağımlılıklarının
  yükseltilmesi.

  ```
  cargo test -p erk-renderer --test chrome_reference -- --nocapture
  ```

  Skor tablosu commit gövdesine ve PR açıklamasına yazılır. Test `cargo test
  --workspace` içinde CI'da da koşar; Chrome gerektirmez.
- **Skor, beklentiye iki ondalıkta tam eşit olmalı; test iki yönde de
  kırılır.** Altında: gerileme, değişiklik birleşmez. Üstünde: beklenti aynı
  commit'te yükseltilir (`tests/reference/expectations.txt`). Aksi halde bir
  iyileşme sonradan hiçbir test fark etmeden geri verilebilirdi.
- **Beklenti düşürmek gerekçe ister:** düşen satırın sonunda
  `# lowered: gerekçe` yorumu olur. CI (`guards` job'ı) beklentileri PR'ın
  tabanıyla karşılaştırır ve yorumsuz düşüşü reddeder. Gerekçe boş olamaz ve
  tabandaki satırda duran, daha önceki bir düşüşün gerekçesi olamaz. Aynı
  sayfa için ikinci bir satır reddedilir (yalnızca ilki okunurdu).
  Yeniden adlandırılan bir Chrome görüntüsü eski adının skorunu taşır.
- **Her yeni render özelliği kendi referans sayfasıyla gelir** (kenarlık,
  görüntü, float, ...), muhafız ilkesiyle aynı gerekçe.
- **Chrome görüntüleri yalnızca yeni sayfa eklenince veya Chrome
  yükseltilince yakalanır**, PR içinde ayrı bir commit'te. PR squash ile
  birleşirse bu ayrım `main`'de kaybolur; bu yüzden Chrome sürümü PR
  açıklamasına da yazılır. Komut yalnızca referansı
  olmayan sayfaları yakalar; Chrome'un sürümü `VERSION.txt`'tekinden farklıysa
  sürüm karıştırmamak için reddeder. Chrome yükseltilince tüm sayfalar
  `ERK_RECAPTURE_ALL=1` ile yeniden yakalanır. Sürüm `chrome.exe`'nin sürüm
  bilgisinden okunur; Windows'ta `chrome.exe --version` açık tarayıcıya
  devredildiği için hiç çağrılmaz.

  ```
  cargo test -p erk-renderer --test chrome_reference -- --ignored capture_chrome_references
  ```

  Bir referans sayfası değişirse Chrome görüntüsü silinip yeniden yakalanır.
  Test bunu zorlar: `chrome/pages.txt` her sayfanın yakalandığı andaki
  özetini tutar ve yakalamadan sonra değişen sayfa testi kırar.
- `tests/reference/pages/` altında yalnızca `.html` dosyaları durur; sayfası
  olmayan bir beklenti ya da Chrome görüntüsü testi kırar.
- Fark görüntüleri ve rapor `target/reference-diff/` altındadır.

## Dokümanlar

- Mimari kararlar: `docs/design/`
- Yol haritası ve uygulama planları: `docs/plans/`
