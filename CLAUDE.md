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
  "proje kuralları" denir.
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
geçtiği ilgili planın "Yürütme Notları"na yazılır.

## Bugün geçerli mimari kurallar

| Kural | Neden | Zorlama | Devreye girdiği yer |
|---|---|---|---|
| `unsafe` yasak | Bellek güvenliği projenin var olma sebebi. İstisna yalnızca adıyla listelenmiş crate'lerde | `[workspace.lints.rust] unsafe_code = "forbid"`, her crate `lints.workspace = true`; istisnalar `unsafe_code = "deny"` yazmak zorunda | M0 Task 1 |
| `erk-style`'ın `unsafe` yüzeyi tam beş imza | Stylo'nun `TElement`'i beş metodu `unsafe fn` tanımlıyor; bunları uygulamak gövde güvenli olsa bile lint ihlali. Başka hiçbir `unsafe` kod yok | CI: `erk-style`'da tam 5 `allow(unsafe_code)`, 0 `unsafe` bloğu | M0 Task 3 |
| `erk-dom` içinde `std::rc::Rc` yok | Döngüsel ağaçta referans sayımı sızıntı üretir; DOM arena + `NodeId` ile çalışır | `crates/erk-dom/clippy.toml` → `disallowed-types`, clippy `-D warnings`; `guards` job'ı lint'in `allow` ile susturulmadığını kontrol eder | M0 Task 2 |
| `erk-dom` projeden hiçbir şey import etmez | En alttaki katman; parser dışında her şey ona bağlanır, o hiçbir şeye | CI'da `cargo tree -p erk-dom` kontrolü | M0 Task 2 |
| Kabuk ile renderer yalnızca mesajla konuşur | M3'te renderer ayrı sürece taşındığında değişen tek şey taşıma katmanı olsun. Paylaşılan değiştirilebilir durum (`Arc<Mutex<Dom>>`) süreç ayrımını yeniden yazıma çevirir | `erk-shell` doğrudan `erk-dom`'a ya da `erk-style`'a bağımlı olamaz (CI'da `cargo tree --depth 1`); `erk-renderer` DOM tiplerini dışa açmaz; mesaj tipleri sahip oldukları veriyi taşır | M0 Task 7 |
| `html5ever` ve `stylo` birlikte yükseltilir | İkisi `web_atoms`/`string_cache` üzerinden aynı atom tiplerini paylaşmak zorunda; html5ever 0.40 ile Stylo 0.21 uyumsuz | `html5ever = "=0.39.0"` sabit; CI'da `web_atoms` ve `string_cache` için tek sürüm kontrolü | M0 Task 3 |

## Kural takvimi

Aşağıdakiler bugün **yok**; korudukları şey geldiğinde gelirler. Tam liste ve
zorlama yöntemi: [p0-verification.md](docs/design/p0-verification.md).

| Taş | Gelen kural |
|---|---|
| M1 | Lisans izin listesi (`cargo deny check licenses`) — Stylo ile ilk MPL-2.0 bağımlılık girer |
| M1 | WPT gerileme yasağı: taban çizgisinin altına düşen PR birleşmez |
| M2 | `erk-renderer` ağ crate'lerine bağımlı olamaz; ağ yalnızca `erk-network` arayüzünden |
| M2 | OpenSSL ve `native-tls` yasak (`cargo deny` bans) |
| M3 | Süreç sınırı: `cargo deny` `wrappers`, `xtask arch-check`, sandbox testi zorunlu check |
| M3 | `unsafe` istisnası: `erk-sandbox` (işletim sistemi API'leri) |
| M4 | `unsafe` istisnası: JS motoru bağlama crate'i (mozjs seçilirse) |

## unsafe ve C/C++ politikası

- `unsafe` yalnızca adıyla listelenmiş crate'lerde bulunur. Bugün liste tek
  kalem: **`erk-style`**, FFI değil ama Stylo'nun trait imzası yüzünden; beş
  `unsafe fn` imzası, gövdeleri güvenli. İstisna crate'i workspace lint'ini devralmaz, kendi `[lints]` tablosunda
  `unsafe_code = "deny"` yazar ve izni öğe bazında `#[allow(unsafe_code)]` ile,
  gerekçe yorumuyla verir.
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
- Planın bir varsayımı yürütmede yanlış çıkarsa, düzeltilmiş gerçek o planın
  "Yürütme Notları"na yazılır; sonraki görevler oradan okunur.

## Referans testi (Chrome karşılaştırması)

Aynı HTML hem Chrome'da hem Erk'te çizilir ve görüntüler piksel piksel
karşılaştırılır (`crates/erk-renderer/tests/chrome_reference.rs`). Altın test
Erk'in kendi çıktısıyla **tam eşitliği** korur; referans testi **Chrome'a
yakınlığı** korur. Piksel piksel aynılık hedef değil (kenar yumuşatma ve
hinting farklı); sayfa başına bir içerik skoru var ve skor yalnızca yükselir.
İçerik pikseli, tuval renginden **herhangi bir** farkı olan piksel; eşleşme
toleransı kanal başına 12 ve referans sayfalarındaki en küçük düz renk farkının
(17) altında kalmak zorunda.

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
  tabanıyla karşılaştırır ve yorumsuz düşüşü reddeder.
- **Her yeni render özelliği kendi referans sayfasıyla gelir** (kenarlık,
  görüntü, float, ...), muhafız ilkesiyle aynı gerekçe.
- **Chrome görüntüleri yalnızca yeni sayfa eklenince veya Chrome
  yükseltilince yakalanır**, ayrı bir commit'te. Komut yalnızca referansı
  olmayan sayfaları yakalar; Chrome'un sürümü `VERSION.txt`'tekinden farklıysa
  sürüm karıştırmamak için reddeder. Chrome yükseltilince tüm sayfalar
  `ERK_RECAPTURE_ALL=1` ile yeniden yakalanır. Sürüm `chrome.exe`'nin sürüm
  bilgisinden okunur; Windows'ta `chrome.exe --version` açık tarayıcıya
  devredildiği için hiç çağrılmaz.

  ```
  cargo test -p erk-renderer --test chrome_reference -- --ignored capture_chrome_references
  ```

  Bir referans sayfası değişirse Chrome görüntüsü silinip yeniden yakalanır.
- `tests/reference/pages/` altında yalnızca `.html` dosyaları durur; sayfası
  olmayan bir beklenti ya da Chrome görüntüsü testi kırar.
- Fark görüntüleri ve rapor `target/reference-diff/` altındadır.

## Dokümanlar

- Mimari kararlar: `docs/design/`
- Yol haritası ve uygulama planları: `docs/plans/`
