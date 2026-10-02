# Erk Engine — proje kuralları

Bu dosya depoda tutulur ve `.gitignore`'a **eklenmez**: kurallar makineye değil
projeye aittir.

## Ürün

**Hedef:** modern, açık kaynaklı, Rust ile güçlendirilmiş yeni nesil bir
Sciter. Sciter'la API ya da CSS uzantısı uyumluluğu hedef değil; ölçüt web
standartları ve Chrome (karşılaştırma: p1-embedded §4).

Erk **gömülü bir HTML/CSS masaüstü UI motorudur**, tarayıcı değil: çekirdekte
JavaScript yok, host uygulama (önce Rust, sonra C-ABI üzerinden Python) DOM'u
`NodeId` ile sürer, Erk çizer ve olayları bildirir. JavaScript isteğe bağlı bir
bağlamadır (`erk-script`, M6): Python gibi genel API'nin üstünde durur,
çekirdeğe girmez. Çekirdekte dosya, ağ, süreç, ortam değişkeni ve saat yoktur;
hepsi host'tan gelir. Gerekçe: [p1-embedded.md](docs/design/p1-embedded.md).

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
- **Çakışmasız PR kuralı.** PR'lar squash ile birleşir: bir dalın commit'leri
  `main`'e başka bir commit olarak girer. Açık bir PR'ın dalı üzerine kurulan
  yeni dal (yığılmış dal), o PR birleşince çakışır. Bu yüzden:
  1. Açık bir PR'ın dalına ait bir düzeltme o dala commit'lenir ve **hemen
     push edilir**; PR birleşmeden önce CI'da görünmesi gerekir.
  2. Sonraki iş mümkünse `main`'den açılır. Önceki PR'a dayanmak zorundaysa
     onun dalından açılabilir, ama o dalın PR'ı açılmadan önce:
  3. Taban PR birleşir birleşmez yığılmış dal yeni `main`'e taşınır:
     `git rebase --onto origin/main <eski taban dalın ucu> <dal>`. Yalnızca
     dalın kendi commit'leri taşınır; tabanın commit'leri `main`'de zaten
     squash olarak durur. Taşınan dalın içeriği, taşımadan önceki haliyle
     `git diff` ile karşılaştırılır; fark yalnızca bilerek eklenen olmalı.
  4. Taşınmış dal `--force-with-lease` ile gönderilir; yalnızca özellik
     dallarına, hiçbir zaman `main`'e.
  5. Yığılmış bir dalın PR'ı, tabanı birleşmeden açılmaz; açıldıysa birleşme
     sonrası 3. adım hemen uygulanır ve PR'ın çakışmasız olduğu denetlenir.

## İnceleme raporları

Her `main` birleşmesinden sonra başka bir inceleyici, birleşen işin
raporunu `docs/reviews/` altına koyar.

- **Her yeni işe başlamadan önce `docs/reviews/`'a bakılır.** Henüz depoda
  izlenmeyen (yeni) bir rapor varsa önce o okunur. Her maddesi
  değerlendirilir: geçerli olanlar o işin parçası olur, geçersiz olanlar
  gerekçesiyle ilgili planın yürütme notlarına yazılır. Ondan sonra işe
  geçilir. Rapor yoksa iş doğrudan başlar.
- Rapor dosyası, onu ele alan işin commit'ine eklenir; böylece depoda
  izlenir ve "yeni rapor" yalnızca izlenmeyen dosya demek olur.
- Commit'e dosyalar adlarıyla eklenir (`git add <yol>`), hiçbir zaman
  `git add -A` ile değil: çalışma dizininde başkasının bıraktığı bir dosya
  okunmadan commit'e girmemeli.

## Dil

| Türkçe | İngilizce |
|---|---|
| `CLAUDE.md`, `docs/design/`, `docs/plans/` | `README.md`, `ARCHITECTURE.md`, `CONTRIBUTING.md`, `docs/css-support.md` (host geliştiricileri için), kod, tanımlayıcılar, yorumlar, commit mesajları, PR'lar |

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
| Kabuk ile renderer yalnızca mesajla konuşur | Mesajlar M3'te C-ABI'nin ve gerekirse ayrı bir sürecin temeli: düz veri, ortak değiştirilebilir durum yok. Paylaşılan durum (`Arc<Mutex<Dom>>`) sınırı yeniden yazıma çevirir | `erk-shell` doğrudan `erk-dom`'a ya da `erk-style`'a bağımlı olamaz (`cargo tree --depth 1 --target all --all-features`). `check-renderer-surface.sh`: `erk-renderer`'ın dış yüzeyi gözden geçirilmiş listeye eşit; mesajlar `messages.rs`'te ve o dosya yalnızca prelude tiplerini kullanır (yol yok, `Arc`/`Mutex`/`Cell`/`Box`/ödünç yok); kabuk `render_html` çağırmaz. `'static` olmayan referansı zaten `spawn`'ın imzası dışlar | M0 Task 7, T8 |
| `html5ever` ve `stylo` birlikte yükseltilir | İkisi `web_atoms`/`string_cache` üzerinden aynı atom tiplerini paylaşmak zorunda; html5ever 0.40 ile Stylo 0.21 uyumsuz | `html5ever = "=0.39.0"` sabit; CI'da `web_atoms` ve `string_cache` için tek sürüm kontrolü (tüm hedefler) | M0 Task 3 |
| Çekirdek G/Ç yapmaz, ortam ve saat okumaz | Gömülü motorda G/Ç host'undur: içerik dosya sistemine ulaşamaz, çekirdek deterministik kalır | `check-core-io.sh`: `erk-dom`, `erk-style`, `erk-renderer` `src`'sinde `std::fs`/`net`/`process`/`env`, `File::`, `TcpStream`, `Command::new`, `Instant::now`, `SystemTime`, fontique'in sistem taraması ve yol yüklemesi yok (yorumlar hariç); `erk-renderer`'ın bağımlılıklarında fontique/Parley `system` özelliği yok. Sistem fontlarını host tarar (p1-contract §6.2) | Yön değişikliği (M0.5 öncesi), M1.7 |
| WPT sonuçları sessizce değişmez | Bir CSS özelliğinin gerçekten çalıştığının dış kanıtı spesifikasyon testleri; geçen bir test sessizce düşerse kimse fark etmez | CI `wpt` job'ı (`erk-wpt check`): sabit WPT commit'indeki reftest sonuçları `tests/wpt/expectations.txt`'ye eşit, iki yönde; `check-wpt-expectations.sh`: PASS'ten düşüş `# lowered:` gerekçesi ister | M1.4 |
| Her bağımlılığın lisansı gözden geçirilmiş listede | Erk kapalı uygulamaların içinde de dağıtılır: atıf dışında yükümlülük getiren (GPL ailesi) bir bağımlılık host'u bağlar. MPL-2.0 (Stylo) dosya düzeyinde kalır, OFL-1.1 gömülü fontlar içindir | CI `licenses` job'ı: `cargo deny --all-features --locked check licenses`, `deny.toml`'daki her izin gerekçeli. `check-license-config.sh`: listeyi dolanan yol (istisna, `clarify`, `private`/`ignore`, `skip`, `exclude`, hedef ya da özellik daraltması, GPL ailesinden izin) yok | M1.6 |
| `render_html` hiçbir girdide paniklemez | Gömülü motor host'unun verdiği her belgeyi çizer; bozuk bir sayfa uygulamayı düşürmemeli | `tests/robustness.rs` (sabit tohumlu üreteç, `tests/robustness/` korpusu, 5000 düzey iç içelik); CI `fuzz` job'ları: sabit nightly ile cargo-fuzz, beşer dakika, paralel: `fuzz (none)` sanitizer'sız (saniyede ~67 girdi, panikler) ve `fuzz (address)` AddressSanitizer'la (~9, sızıntı ve bellek hataları); çöken girdi artifact olarak saklanır ve korpusa girer (kasıtlı bir panikle doğrulandı) | M1.3, M1 |
| İkili boyutu bütçenin altında | Gömülü bir motorun boyutu ürünün boyutu; bütçe ölçümden konur, gerekçesiz yükselmez | CI `size` job'ı (`check-size-budget.sh`): Linux yayın ikilisi `.github/size-budget.txt`'teki tavanın altında; yükseltme `# raised:` gerekçesi ister | M1.0 |
| CSS matrisindeki her "Supported" satırın testi var | `css-support.md` host geliştiricisine verilen söz; testsiz bir "Supported" söz değildir | CI `guards` (`check-css-support.sh`): her Supported satır var olan bir testi adlandırır | M1.0 |
| CI kilit dosyasıyla derler | Altın görüntüler ve Chrome skorları `Cargo.lock`'taki sürümlerle üretildi; kilitten sapan bir manifest CI'da sessizce yeniden çözülmemeli | clippy, build, test ve `cargo tree` adımlarında `--locked` | M0 Task 8 |

## Kural takvimi

Aşağıdakiler bugün **yok**; korudukları şey geldiğinde gelirler. Tam liste ve
zorlama yöntemi: [p0-verification.md](docs/design/p0-verification.md).

| Taş | Gelen kural |
|---|---|
| M0.5 | Sözleşmenin her kuralı hangi taşta hangi muhafızla zorlanacağını söyler (p1-contract.md) |
| M3 | `unsafe` istisnası: `erk-ffi` (C-ABI); üretilen `erk.h` depodakiyle aynı; C örneği CI'da derlenip çalışır |
| M3 | FFI'dan panik sızmaz; eski `NodeId` ve yanlış iş parçacığı hata kodu döner |
| M4 | `Mutation` dizileri fuzz'lanır |
| M5 | `erk-invalidation` projeden yalnızca `erk-dom`'a bağımlı; artımlı her yol tam yeniden hesapla aynı display list'i verir (fuzz) |
| M6 | Çekirdek crate'ler, `erk` ve `erk-ffi` hiçbir JS motoruna bağımlı değil; `erk-script` projeden yalnızca `erk`'e bağımlı (`cargo tree`) |

## unsafe ve C/C++ politikası

- `unsafe` yalnızca adıyla listelenmiş crate'lerde bulunur. Bugün liste tek
  kalem: **`erk-style`**, FFI değil ama Stylo'nun trait imzası yüzünden; M3'te
  **`erk-ffi`** eklenir (C-ABI ham işaretçi ister; `#[unsafe(no_mangle)]`). Beş
  `unsafe fn` imzası, gövdeleri güvenli. İstisna crate'i workspace lint'ini
  devralmaz, kendi `[lints]` tablosunda `unsafe_code = "deny"` ve
  `unsafe_op_in_unsafe_fn = "forbid"` yazar ve izni öğe bazında
  `#[allow(unsafe_code)]` ile, gerekçe yorumuyla verir.
- Bugün C/C++ bağımlılığı yok (işletim sisteminin pencere ve grafik
  kütüphaneleri dışında). Yeni bir C/C++ bağımlılığı bir tasarım kararıdır ve
  `docs/design/` altına yazılır.

## Derleme önkoşulları

- Rust stable; sürüm `rust-toolchain.toml`'da sabit.
- Windows: MSVC Build Tools (C++ iş yükü).
- Python 3: Stylo'nun `build.rs`'i `properties/build.py`'yi çalıştırır. Önce
  `PYTHON3` ortam değişkenine, yoksa Windows'ta `python.exe`'ye bakar.
- LLVM/libclang **gerekmez**: Stylo'da bindgen yalnızca `gecko` özelliğinde.

## Gizlilik ve güvenlik

- **Telemetri yok, ağ yok.** Motor hiçbir ağ isteği atmaz ve varsayılan olarak
  hiçbir portu dinlemez (inspector dahil).
- **G/Ç yalnızca host'ta.** Çekirdek crate'ler (`erk-dom`, `erk-style`,
  `erk-renderer`) dosya, ağ, süreç, ortam değişkeni ve saat kullanmaz;
  kaynaklar host'un callback'inden, zaman host'un `now_ns`'inden gelir.
  İçerikteki `url("file:///etc/passwd")` hiçbir şey okuyamaz. Çekirdek
  `<script>` çalıştırmaz; betik ancak host isteğe bağlı `erk-script`'i
  açarsa çalışır ve host'un açtığı işlevler dışında hiçbir şeye erişemez.
- Loglara sayfa içeriği, form verisi veya kimlik bilgisi yazılmaz.

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
- Mimari tartışma yalnızca o anki taşı bloke ediyorsa yapılır; etmiyorsa
  ilgili taşın planına açık soru olarak yazılır.
