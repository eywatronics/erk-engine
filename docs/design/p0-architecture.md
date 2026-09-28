# Erk Engine — P0 Tasarım Dokümanı

- **Tarih:** 2026-09-25
- **Durum:** Onaylandı
- **Kapsam:** Motorun hedef mimarisi, M0–M3 arasındaki sıralama kararı ve ilk
  kilometre taşlarının teknoloji seçimleri

---

## 1. Ürün tanımı

Erk, Rust ile yazılan, çoklu süreçli, kum havuzlu bir **tam masaüstü tarayıcı
motoru**dur. Hedef platformlar sırasıyla Windows, Linux ve macOS.

Erk bir "sıfırdan her şey" projesi değildir. Olgun Rust bileşenlerini
(html5ever, Stylo, Taffy, Parley, Vello ailesi) kullanır; özgün iş, hiçbir
hazır parçanın kapsamadığı yerlerdedir:

- satır içi biçimlendirme bağlamı (inline formatting context) ve akış layout'u
- süreç modeli, IPC ve kum havuzu
- Fetch standardı ve ağ güvenlik politikası
- tarayıcı kabuğu

Bu doküman süre tahmini içermez. Kilometre taşları kapsam ve kabul kriteriyle
tanımlanır ([roadmap.md](../plans/roadmap.md)).

---

## 2. Onaylanmış kararlar

| Karar | Seçim | Gerekçe |
|---|---|---|
| Strateji | Kendi motor, hazır parçalar | Servo'yu gömmek en hızlı kullanılabilir tarayıcıyı verirdi ama Erk bir motor olmazdı (§2.3). |
| Sıralama | **Önce piksel, sonra izolasyon** | Tek süreç ve yerel dosyayla başlanır; süreç ayrımı M3'te. Gerekçe §2.1. |
| Süreç sınırına hazırlık | Kabuk ↔ renderer yalnızca tipli mesajla (`std::sync::mpsc`), renderer ayrı iş parçacığında | M3'te değişen şey yalnızca taşıma katmanı olur. §2.2. |
| HTML ayrıştırma | `html5ever = "=0.39.0"` | 0.40, `web_atoms` 0.3 / `string_cache` 0.11'e geçti; Stylo 0.21 hâlâ `web_atoms` 0.2 / `string_cache` 0.9 kullanıyor. Atom tipleri farklı olur ve Stylo'nun `TElement::local_name()` gibi metotları derlenmez. Blitz da 0.39'a sabitli. |
| DOM | Kendi arena DOM'umuz (`erk-dom`), `NodeId` = u32 indeks + u32 nesil | `markup5ever_rcdom` kendi README'sinde üretim kalitesinde olmadığını söylüyor ve `Rc` tabanlı. §5. |
| Stil | `erk-style` crate'inde `stylo` (`servo` özelliği), M0–M1'de paralellik kapalı. M0'da uyarlanan Blitz adaptörünün derlendiği sürüm (blitz-dom 0.3.0-beta.2 → 0.20.x); 0.21'e yükseltme M0'dan sonra ayrı PR | Servo ve Firefox'un CSS motoru; crates.io'da yayımlanıyor. Adaptörü birebir alabilmek için onun sürümünde başlanır. Paralellik isteğe bağlı: `traverse_dom`'a rayon havuzu verilmezse sıralı çalışır. |
| Layout | Taffy 0.14 (low-level API) + kendi IFC | Taffy block, flexbox, grid, float/clear destekliyor; tablo, inline ve metin layout'u yok. §6.2. |
| Metin | Parley 0.11 (HarfRust + ICU4X + fontique) | rustybuzz arşivlendi ve yerini HarfBuzz'ın resmî Rust portu HarfRust aldı; Parley 0.6'dan beri onu kullanıyor. |
| Boyama | Erk'in kendi display list'i → `vello_cpu` 0.2 | Deterministik PNG, GPU kurulumu yok, reftest için ideal. GPU yolu (`vello_hybrid`, wgpu 29) M2'de. §6.3. |
| Pencere | winit 0.30 + softbuffer 0.4 | servoshell de winit 0.30 kullanıyor. 0.31 hâlâ beta. |
| Ağ | M2'de geçici reqwest (arayüz arkasında), M6'da hyper 1.x üstünde kendi Fetch'imiz | reqwest'in havuz anahtarı `(scheme, authority)`: Fetch'in istediği partition key, SameSite ve cache bölümlemesi ifade edilemiyor. Ama gezinmek için yeterli ve Fetch'i erkenden yazmak M2'yi aylarca uzatırdı. §7. |
| TLS | rustls 0.23 + `aws-lc-rs` | Protokol Rust, kripto C/asm. Kural "saf Rust" değil, "OpenSSL/native-tls yok". |
| JavaScript | M4. Aday sırası: Boa, sonra mozjs. V8 yok | §8. |
| IPC | M3'te ölçülerek: `ipc-channel`+serde vs `rkyv` | rkyv'nin "sıfır kopya" vaadi, güvenilmeyen süreçten gelen veride doğrulama (bytecheck) maliyetiyle birlikte ölçülmeli. |
| Lisans | MIT OR Apache-2.0; MPL-2.0 bağımlılıklar kabul | Stylo, selectors ve mozjs MPL-2.0: dosya bazında copyleft. Bağımlılık olarak kullanmak Erk'in lisansını etkilemez; **kopyalanıp değiştirilen** bir MPL dosyası MPL kalır. |

### 2.1 Önce piksel, sonra izolasyon

İlk taslak planda M0 süreç iskeleti, IPC ölçümü, Windows sandbox'ı ve mimari
muhafızlardı. Eleştiri haklıydı: bu, ekranda tek piksel yokken yangın merdiveni
yönetmeliği tartışmaktı. Mimarinin nerede esneyeceğini görmeden konan katı
sınırlar, DOM ile layout arasında hızlı prototip gereken yerde pranga olur.
Ladybird de böyle başladı: tek süreç, pencere, ayrıştırma, piksel.

Yeni sıra: **piksel (M0) → statik belge motoru (M1) → gezinme (M2) → süreç
ayrımı ve sandbox (M3) → JavaScript (M4)**.

### 2.2 Eleştirinin görmediği risk ve bedeli sıfıra yakın önlem

Çoklu süreci sonradan eklemek pahalıdır. Firefox'un tek süreçten çoklu sürece
geçişi (Electrolysis) yıllar sürdü, çünkü kod tabanı her yerde paylaşılan
değiştirilebilir duruma dayanıyordu.

Önlem tek bir disiplin: **kabuk ile renderer arasında paylaşılan değiştirilebilir
durum yok.** Renderer M0'dan itibaren kendi iş parçacığında çalışır, kabukla
yalnızca tipli mesajlarla konuşur. Mesaj tipleri sahip oldukları veriyi taşır —
referans, `Arc`, `Mutex` yok. M3'te `mpsc::channel` yerine IPC kanalı takılır;
mesajlar zaten sahipli veri olduğu için serileştirilebilir hale gelmeleri bir
türetme satırıdır.

Bu disiplin CI'da zorlanır: `erk-shell`'in doğrudan bağımlılıkları arasında
`erk-dom` yok ve `erk-renderer` DOM tiplerini dışa açmaz. DOM'a dokunamayan bir
kabuk, DOM'u paylaşamaz.

### 2.3 Reddedilen alternatifler

- **Servo'yu gömmek (`servo` crate, WebView API).** Servo 0.1.0 2026-04-13'te
  crates.io'ya çıktı, 0.1.x LTS hattı var. Tek kişi için en hızlı kullanılabilir
  tarayıcı yolu buydu. Reddedildi çünkü Erk bir kabuk olurdu, motor değil; motor
  düzeltmeleri Servo'ya katkı olarak giderdi. Bu bilinçli bir tercih:
  bedeli, ilk kullanılabilir tarayıcının çok daha geç gelmesi.
- **Blitz'i çekirdek olarak kullanmak (`blitz-dom`'a bağımlılık).** Piksel
  görmenin en hızlı yolu. Reddedildi çünkü Erk'in DOM'u Blitz'in DOM'u olurdu ve
  Blitz beta (0.3.0-beta.2), sık değişiyor, Stylo'nun bir sürüm gerisinde.
  Blitz **referans ve test kâhini** olarak kullanılır; `stylo.rs` adaptörü
  kaynak gösterilerek uyarlanır.
- **Önce çoklu süreç (ilk taslak).** §2.1.
- **reqwest'i kalıcı ağ katmanı yapmak.** §7.
- **Compute tabanlı `vello` crate'i.** 0.10 araştırma statüsüne taşınıyor
  (`vello_research`), compute shader istiyor, bellek ayırma sorunları açık.
- **V8 (`v8` crate).** §8.
- **Takılabilir JS motoru soyutlaması.** Gosub'un V8 ve web API crate'leri
  derleniyor ama README'sine göre hiçbir sayfa JavaScript çalıştırmıyor.
  Soyutlama, iki motoru da yarım bağlamanın yoludur.
- **Kendi font ayrıştırıcı, shaper, görüntü çözücü veya metin kodlayıcımız.**
  skrifa/read-fonts, HarfRust, png/jxl-rs ve encoding_rs var; Chromium bile
  bunları kullanıyor.

---

## 3. Hedef mimari

```
┌──────────────────────────────┐
│  Kabuk / broker (erk-shell)  │  yetkili: pencere, girdi, süreç yaşam döngüsü
└──────┬───────────────┬───────┘
       │ IPC           │ IPC
┌──────▼───────┐ ┌─────▼────────┐
│  Renderer    │ │  Ağ süreci   │  soketler, DNS, TLS, çerezler, cache
│ (kum havuzu, │ │ erk-network  │
│ site başına) │ └──────────────┘
└──────────────┘
```

- **Kabuk** tek yetkili süreçtir. Pencereyi, girdiyi ve diğer süreçleri yönetir.
- **Renderer** site başına (şema + eTLD+1; origin değil) ayrı süreçtir. Diske
  ve ağa erişimi yoktur. Chromium da siteye göre ayırır.
- **Ağ süreci** soketlerin, TLS'in, çerezlerin ve cache'in tek sahibidir. CORS,
  CORP ve ORB denetimi burada, baytlar renderer'a geçmeden yapılır.

### Bugün (M0)

Tek süreç. Kabuk ana iş parçacığında pencereyi çalıştırır; renderer ayrı bir iş
parçacığında DOM'u, stili, layout'u ve boyamayı yapar; ikisi `mpsc` ile konuşur.
Ağ yok, yerel dosya okunur.

---

## 4. Crate'ler ve bağımlılık yönü

```
erk-shell ──► erk-renderer ──► erk-style ──► erk-dom
    │               └──────────────────────────▲
    └──► erk-network   (M2)
```

| Crate | Sorumluluk | Bugün |
|---|---|---|
| `erk-dom` | Arena DOM, `NodeId`, html5ever `TreeSink` | M0 Task 2 |
| `erk-style` | Stylo adaptörü: `TElement` ve arkadaşları, yan tablo, `StyleEngine`. Tek `unsafe` istisnası (§6.1) | M0 Task 3 |
| `erk-renderer` | Layout (Taffy + Parley), display list, boyama | M0 Task 4–6 |
| `erk-shell` | Pencere, olay döngüsü, renderer iş parçacığıyla mesajlaşma | M0 Task 7 |
| `erk-network` | Ağ arayüzü; M2'de reqwest, M6'da kendi Fetch | Boş |
| `erk-ipc` | Süreçler arası taşıma | M3 |
| `erk-sandbox` | İşletim sistemi kum havuzu API'leri (tek `unsafe` istisnası) | M3 |
| `erk-js` | JS motoru bağlama | M4 |

`erk-dom` yapraktır: projeden hiçbir şey import etmez. `erk-shell`, `erk-dom`'a
bağımlı olamaz (§2.2).

---

## 5. DOM bellek modeli

### 5.1 Arena ve `NodeId`

Düğümler tek bir `Vec<Slot>` içinde ardışık durur. Düğümler birbirini işaretçiyle
değil `NodeId` ile gösterir:

```rust
pub struct NodeId {
    index: u32,
    generation: NonZeroU32,
}
```

- Bir düğüm silindiğinde slot'u serbest listeye girer ve nesli artar. Eski bir
  `NodeId` ile erişim `None` döner, başka bir düğümü göstermez.
- **İlk plandaki "üst 8 bit nesil, alt 24 bit indeks" reddedildi:** nesil 256
  silmede sarar (aynı slot'a eski `NodeId` yeniden geçerli olur, klasik ABA) ve
  indeks 16 milyon düğümde tavan yapar. İki ayrı u32'nin bedeli, tek u32'ye
  göre `NodeId` başına dört bayt fazlası; `NonZeroU32` sayesinde
  `Option<NodeId>` da sekiz baytta kalır.
- `std::rc::Rc` `erk-dom`'da yasak (clippy `disallowed-types`).

### 5.2 İç değiştirilebilirlik

html5ever 0.39'un `TreeSink` metotları `&self` alıyor. Arena bu yüzden iç
değiştirilebilirlik ister (`RefCell` ya da hücre bazında `Cell`). `RefCell`
yasak değildir; yasak olan düğüm sahipliğini referans sayımıyla kurmaktır.

### 5.3 JS ile sahiplik (M4'te karar verilecek)

Arena ile JS çöp toplayıcısı arasındaki sahiplik kuralı M4'te açıkça seçilir:
ya JS düğümlerin sahibidir (Servo modeli: SpiderMonkey'nin GC'si DOM
nesnelerini tutar), ya da arena sahibidir ve arenadan erişilebilen JS kenarları
bir kökten izlenir. Hangisi seçilirse seçilsin kabul testi aynı:

> Ayrılmış (detached) bir alt ağaç, kendisini kapanışında (closure) tutan bir
> olay dinleyicisiyle birlikte toplanır.

İlk plandaki "V8'in GC'si Rust'a nesnenin canlı olup olmadığını soracak" fikri
geçersiz: V8 `EmbedderHeapTracer`'ı 2022'de kullanımdan kaldırdı ve sonra sildi;
böyle bir kanca yok.

---

## 6. Render hattı

```
HTML ─► html5ever ─► erk-dom ─► Stylo ─► Taffy + Parley ─► display list ─► vello_cpu ─► PNG / softbuffer
```

### 6.1 Stil

Stylo, gömücüden `TNode`, `TElement`, `TDocument`, `TShadowRoot`, `NodeInfo` ve
`selectors::Element` uygulamalarını ister; `TElement` tek başına 46 zorunlu
metot. Blitz'in `blitz-dom/src/stylo.rs`'i (~1,5 bin satır) bunların hepsini tek
bir `Copy` tutamakta uyguluyor. **Bu katmanda özgünlük aranmaz:** Blitz'in
adaptörü arena `NodeId`'ye birebir uyarlanır, kaynak gösterilir. Shadow DOM
stub olarak kalır.

Stylo her ay bir 0.x sürümü çıkarıyor ve her biri kırıcı sayılmalı. Yükseltme
html5ever ile birlikte yapılır.

Adaptör kendi crate'inde, `erk-style`'da durur, çünkü iki kısıt yürütmede
ortaya çıktı (ayrıntı: M0 planının Task 3 yürütme notları):

- **`TElement` beş metodu `unsafe fn` tanımlıyor.** Bunları uygulamak, gövde
  güvenli olsa bile `unsafe_code` ihlali; `forbid` altında hiç yapılamaz.
  `erk-style` `deny` seviyesinde, izin yalnızca bu beş imzada, gövdeler güvenli.
  CI sayıyı sabit tutar.
- **Tutamak tam bir işaretçi genişliğinde olmalı.** Stylo'nun stil paylaşım
  önbelleği eleman tipini `transmute` ile siliyor. Tutamak `&StyledNode`'dur;
  her kayıt kendi `NodeId`'sini ve ağaca bir referansı tutar. Boyut derleme
  zamanında doğrulanır.

Stylo'nun düğüm başına verisi `erk-dom`'da değil bu crate'in yan tablosundadır;
Blitz'in yaptığı gibi düğüme ağaca işaret eden ham bir işaretçi koymak
`erk-dom`'a `unsafe` sokardı.

### 6.2 Layout

Taffy'nin low-level API'si kullanılır (`LayoutPartialTree` ve ilgili trait'ler):
Erk'in DOM'u layout ağacıdır, ayrı bir `TaffyTree` kopyası tutulmaz. Stylo'nun
hesaplanmış değerleri `stylo_taffy` ile Taffy stiline çevrilir (üçlü lisanslı;
MIT OR Apache-2.0 altında kullanılıyor).

`stylo_taffy`, `calc()` değerlerini Taffy'ye ham işaretçi olarak geçirir ve
Taffy çözümleme için işaretçiyi geri verir. Blitz onu `unsafe` ile izler.
Erk izlemez: layout ağacı kurulurken `calc()` değerleri adresleriyle bir
tabloya kopyalanır, işaretçi yalnızca anahtar olur (`erk-renderer/src/layout/calc.rs`).

**Özgün iş: inline formatting context.** Taffy satır kutularını, span
kırılmasını, `text-align: justify`'ı, `vertical-align`'ı ve satır içi
görselleri bilmez. Bunu Erk yazar. Referanslar: Servo'nun `layout` crate'i
(eski adı layout_2020) ve Blitz'in `layout/inline.rs`'i.

- **Metin M0'da gömülü Noto Sans ile çizilir** (Regular + Bold, OFL-1.1),
  sistem yazı tipleri yüklenmez: ölçüm ve çizim her makinede aynıdır. Stylo'nun
  `ex`/`ch` gibi birimleri için yazı tipi ölçümlerini `erk-style` bilmez;
  `erk-renderer` aynı gömülü fonttan okuyan bir sağlayıcıyı dışarıdan verir.
- `line-height: normal`, Chrome'un yaptığı gibi fontun ascent, descent ve
  line gap değerlerini ayrı ayrı tam piksele yuvarlayıp toplar. Yuvarlamadan
  her satır ~0.2px kısa kalıyor ve fark sayfa boyunca birikiyordu (Chrome
  referans testi buldu).
- **M0'da** tam IFC yok: bir paragraf Taffy'de ölçüm fonksiyonlu bir yapraktır.
  Parley paragrafı şekillendirip satırlara böler, Taffy'ye yalnızca
  `(genişlik, yükseklik)` döner; aynı Parley layout'u boyamada tekrar
  kullanılır.
- **M1'de** tam IFC.

**Ölçülerek verilecek karar — akış layout'unun sahibi.** Taffy block layout ile
başlanır. M1'de `css/CSS2/normal-flow` ve `css/CSS2/floats` taban çizgisi Taffy'nin
sınırlarında tıkanırsa (float ile satır kutusu etkileşimi, margin collapsing
köşe durumları), akış layout'u Servo modeline taşınır: block ve inline Erk'in,
Taffy yalnızca flex ve grid'de. Bu karar "tartışılarak" değil, taban çizgisinde
hangi testlerin neden kaldığına bakılarak verilir.

### 6.3 Boyama ve display list

Boyama sırası CSS 2 Ek E'ye uyar: tek yığın bağlamında önce tüm blok arka
planları (ağaç sırasıyla), sonra tüm metin. Tuval rengi beyaz bir tabanın
üstüne harmanlanır, kare her zaman opaktır; kutu üretmeyen (`display: none`)
kök ya da body tuvale renk yaymaz. `visibility: hidden` kutuyu tutar ama
boyamaz.

Display list Erk'indir ve webrender_api'nin çizgisindedir, Blitz'in her karede
DOM'u yeniden gezen yaklaşımında değil: düz, serileştirilebilir öğeler
(dikdörtgen, kenarlık, gölge, glyph run, görüntü, gradyan), bunların işaret
ettiği yan tablolar (uzamsal ağaç: kaydırma çerçeveleri ve dönüşümler; kırpma
zincirleri; yığın bağlamları). M0'da yalnızca dikdörtgen ve glyph run var.
Display list reftest için metin olarak da dökülebilir.

`vello_cpu` varsayılan ve referans boyayıcıdır. GPU yolu M2'de `vello_hybrid`
(wgpu **29**; vello_hybrid ve Blitz 30'a geçmedi) ile gelir; adaptör veya yüzey
oluşturulamazsa CPU'ya düşülür.

**"4K'da 120 FPS garantisi" iddiası kaldırıldı.** Böyle bir garanti hiçbir
kaynakta yok. Akıcı kaydırma boyayıcının değil kompozitörün işidir (kaydırma
katmanı başına tile cache, kompozitör iş parçacığında kaydırma) ve Vello ailesi
bunu sağlamıyor; M9'da Erk yazar. Hedef, adlandırılmış bir sayfa kümesinde
ölçülen p95 kare süresidir.

---

## 7. Ağ ve güvenlik modeli (hedef)

M2'deki ağ yalnızca gezinme içindir: `erk-network` bir arayüz sunar, arkasında
geçici olarak reqwest (rustls/aws-lc-rs, yönlendirme, gzip/br) çalışır. JS
olmadığı için CORS henüz anlam taşımaz.

M6'da reqwest, hyper 1.x bağlantı API'si üstünde yazılan kendi Fetch
uygulamamızla değiştirilir. Tasarım bugünden şunları öngörür:

- Bağlantı havuzu Fetch'in istediği anahtarla: `(network partition key, origin,
  credentials)`. hyper-util'in legacy istemcisi `(scheme, authority)` kullanıyor.
- İstek tipinde partition key, credentials modu, istek modu (cors / no-cors /
  navigate), hedef ve initiator origin **zorunlu alanlardır**. Initiator origin
  renderer'ın süreç kilidine karşı ağ sürecinde doğrulanır; renderer'ın iddia
  ettiği origin'e asla güvenilmez.
- Çerezler kendi RFC 6265bis kavanozumuzla (SameSite, `__Host-`/`__Secure-`,
  CHIPS). `cookie_store` ve reqwest'in `CookieStore`'u yalnızca URL alıyor;
  site-for-cookies bilgisi olmadan SameSite uygulanamaz.
- CORB, Fetch'ten 2022'de kaldırıldı; yerini ORB aldı (annevk/orb açıklaması,
  Chrome ve Firefox'ta var, spesifikasyonda henüz yok). Denetim ağ sürecinde
  yapılır — ilk plandaki "işletim sistemi düzeyinde" ifadesi yanlıştı.
- HTTP/3 ertelendi: `h3` 0.0.8 kendini "çok deneysel" diye tanımlıyor.

---

## 8. JavaScript (M4)

M4 bir spike değil, bir kilometre taşıdır; motoru bağlamak ile döngü toplayan
bir DOM-GC mimarisi kurmak ayrı işlerdir.

| Aday | Durum | Artı | Eksi |
|---|---|---|---|
| **Boa** 0.22 | Saf Rust, Test262 %95,6 | Tek GC (`Trace`/`Finalize`), C++ yok, döngüyü kendi toplayıcısı çözer | Üçüncü taraf bir Octane benzeri ölçümde V8'den ~180 kat yavaş; GC'si "prototip" |
| **mozjs** 0.26 (SpiderMonkey ESR 153) | Servo'nun motoru | Rust DOM'u JS GC'ye bağlamanın tek tam açık kaynak referansı (JSClass trace hook, `JSTraceable`); Windows için hazır arşiv (~90 MB) | Sık kırılan sürümler, MPL-2.0, Servo'nun kök güvenliği nightly'ye bağlı `crown` lint'ine dayanıyor, hazır arşivin Windows'ta LLVM isteyip istemediği doğrulanmadı |
| ~~V8~~ | Reddedildi | En hızlısı | Rust üzerinde kurulmuş olgun bir DOM örneği yok; cppgc bağları neredeyse tamamen `unsafe`; her ~4 haftada kırıcı sürüm |

Aday sırası Boa, sonra mozjs. Karar M4 başında aynı mini DOM üzerinde ölçülerek
verilir. Ölçülenler:

- temiz bir Windows'ta gereken önkoşullar ve soğuk derleme süresi
- GC doğruluğu: §5.3'teki döngü testi; zorla GC sonrası `el === el`
- 10 bin oluştur/ayır döngüsünde bellek büyümesi
- binding çağrısı başına süre
- motoru bir sürüm yükseltmenin maliyeti

---

## 9. Test stratejisi

- Birim testleri her crate'te.
- Altın PNG testleri: `render_html` çıktısı depodaki referansla piksel piksel
  karşılaştırılır (kabuğun `--screenshot` yolu aynı işlevi çağırır).
- Chrome referans testi: aynı sayfalar Chrome'da ve Erk'te çizilir, sayfa
  başına içerik skoru iki ondalıkta sabitlenir ve yalnızca gerekçeyle düşebilir.
- WPT (M1'den itibaren): wptrunner'a `erk --screenshot` üzerinden koşan özel bir
  "erk" ürünü eklenir (Servo'nun `executorservo` yaklaşımı). Önce reftest'ler;
  testharness.js testleri JS ile (M4) gelir.
- Taban çizgisi JSON olarak yayımlanır, DioxusLabs `browser-wpt-results` ile
  aynı biçimde (`tests, score, subtests, passed`), böylece Blitz, Servo ve
  Ladybird ile doğrudan karşılaştırılabilir.
- WPT kapısı "her test geçmeli" değil, **"beklentinin altına düşme"**dir.
  Hiçbir motor WPT'nin tamamını geçmiyor; Chrome bile css/CSS2'de ~%98.

---

## 10. Riskler

| Risk | Etki | Hafifletme |
|---|---|---|
| Tek geliştirici + AI | Zamanın çoğu kod yazmak değil, spesifikasyon okumak ve kütüphaneler arası hataları ayıklamak | Taşlar küçük; her taş gösterilebilir bir çıktıyla biter; süre tahmini verilmez |
| Stylo'nun aylık kırıcı sürümleri | Her yükseltme adaptörü kırabilir | html5ever ile birlikte, ayrı PR'da yükseltme; tek sürüm kontrolü CI'da |
| Atom sürüm kayması | html5ever 0.40'a geçilemiyor | `=0.39.0` sabit; Stylo `web_atoms` 0.3'e geçince birlikte |
| Blitz/Taffy bus faktörü | Blitz, Taffy ve `stylo_taffy` büyük ölçüde Nico Burns'e bağlı; Dioxus Labs 2026-09-10'da Cognition'a katıldı | Blitz referans olarak kullanılıyor, bağımlılık olarak değil; Taffy'den çıkış yolu §6.2'de |
| IFC'nin büyüklüğü | Tarayıcının en çok köşe durumu barındıran yeri | M0'da ertelenir, M1'in merkezi; Servo `layout` referans |
| Vello ailesinin kararsızlığı | `vello_hybrid` → `vello_gpu` yeniden adlandırılıyor; `vello_cpu` API'sini yeniden tasarlayacak; filtre grafikleri panikliyor | Boyayıcı Erk'in display list'inin arkasında; değişimin etkisi tek modülde |
| MPL-2.0 | Değiştirilen Stylo dosyaları MPL kalır | Stylo'yu çatallamamak; bağımlılık olarak kullanmak |
| Çoklu süreci sonradan eklemek | e10s benzeri yeniden yazım | §2.2 mesaj disiplini, CI'da zorlanır |

---

## 11. Kaynaklar

Sürümler 2026-09-25 itibarıyla crates.io'dan doğrulandı.

- html5ever 0.40.1 / 0.39.0 — https://github.com/servo/html5ever
- stylo 0.21.0 (M0'da 0.20.x) — https://github.com/servo/stylo
- Blitz 0.3.0-beta.2 — https://github.com/DioxusLabs/blitz
- taffy 0.14.0 — https://github.com/DioxusLabs/taffy
- parley 0.11.1, fontique 0.11.1 — https://github.com/linebender/parley
- harfrust 0.13.3 — https://github.com/harfbuzz/harfrust
- vello_cpu 0.2.0, vello_hybrid 0.2.0 — https://github.com/linebender/vello
- wgpu 30.0.1 (Vello ailesi 29'da) — https://github.com/gfx-rs/wgpu
- winit 0.30.13 — https://github.com/rust-windowing/winit
- boa_engine 0.22.0 — https://github.com/boa-dev/boa
- mozjs 0.26.3 — https://github.com/servo/mozjs
- hyper 1.11.1, reqwest 0.13.5, rustls 0.23.45
- Servo'nun WebView API'si: https://servo.org/blog/2026/04/13/servo-0.1.0-release/
- WPT karşılaştırmaları: https://github.com/DioxusLabs/browser-wpt-results
- ORB: https://github.com/annevk/orb
