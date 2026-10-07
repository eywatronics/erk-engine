# Erk Engine — P1: Gömme sözleşmesi (M0.5)

- **Tarih:** 2026-09-30
- **Durum:** ABI v0.2, M3.5'te koda indi (`crates/erk-ffi`,
  `include/erk.h`, §10). Değişiklik önce bu belgede, gerekçesiyle yapılır;
  M4–M5 boyunca öğrenilenlerle v0.3 diye ilerler; 1.0'a (M8) kadar kırıcı
  değişiklik hakkı saklıdır.
- **Üst belge:** [p1-embedded.md](p1-embedded.md)

Bu belge, host uygulamayla Erk arasındaki sınırın kurallarını kod yazılmadan
sabitler: tipler, bellek sahipliği, iş parçacığı modeli, callback ömrü, hata
modeli, sürümleme. Amaç, M1 ve M2'de yazılacak Rust çekirdeğinin M3'te C-ABI'ye
uydurulmak için yeniden yazılmak zorunda kalmaması.

Kurallar Rust API'si (`erk`) ve C-ABI (`erk-ffi`) için aynıdır; C-ABI aynı
API'nin düz hâlidir. Aşağıdaki `erk.h` elle yazılmış bir taslaktır; M3'te
cbindgen ile üretilir ve CI üretilenin depodakiyle aynı olduğunu denetler.

---

## 1. Kararlar

### 1.1 Belge UI iş parçacığında, raster ayrı iş parçacığında

M0'da renderer iş parçacığı her şeyi yapıyor: ayrıştırma, stil, layout,
boyama. Gömme API'si bunu değiştirir. `erk_node_create` bir `NodeId`'yi hemen
döndürmeli, `erk_node_text` gibi sorgular hemen yanıt vermeli. Belge başka bir
iş parçacığındaysa her çağrı bir gidiş-dönüş olur ya da id'ler iki tarafta
ayrı ayrı üretilmek zorunda kalır.

Karar: **DOM, stil ve layout, `ErkApp`'le birlikte UI iş parçacığında durur.**
Değişiklikler anında uygulanır, sorgular anında yanıt verir. Kare başına bir
kez, bekleyen değişikliklerden sonra stil ve layout çalışır ve bir display
list üretilir. **Raster (vello) ayrı iş parçacığındadır** ve yalnızca display
list alır, kare döndürür. Tarayıcıların ana iş parçacığı ile raster/kompozitör
ayrımı da budur.

Sonuçları:

- M0'daki "kabuk ile renderer yalnızca düz veri mesajla konuşur" kuralı, UI
  iş parçacığı ile raster iş parçacığı arasındaki sınıra taşınır. Display list
  düz veri olmalı. M3.0'dan beri öyle: glyph run'lar yüzü `FontId`, görüntü
  öğeleri pikselleri `ImageId` ile anar; baytlar raster tarafındaki font ve
  görüntü tablolarına bir kez, ilk kullanan listeden önce gider
  (`erk-renderer/src/list.rs`).
- Layout'un maliyeti UI iş parçacığındadır; host'un olay işleyicileriyle aynı
  iş parçacığı. Artımlı stil ve layout (M5) bu yüzden önemli; M2'ye kadar
  tam yeniden hesaplama bilerek kabul edilir.
- `erk-renderer`'ın bugünkü `spawn()`/`ToRenderer::Load` yolu M0'ın demo
  yoludur. M3'te ayrıştırma, stil ve layout `erk` tarafına geçer, iş parçacığı
  raster'a iner. `check-renderer-surface.sh` o PR'da yeni sınıra göre yeniden
  yazılır.

### 1.2 Süreç içi, tek belge, tek pencere

Bir `ErkApp` bir pencere ve bir belge demektir. Birden fazla pencere, birden
fazla `ErkApp` ile olur; aynı iş parçacığında olmaları gerekir (winit'in tek
olay döngüsü). Ayrı süreç modu planlanmıyor.

### 1.3 Çekirdek saftır

Çekirdek (`erk-dom`, `erk-style`, `erk-renderer`) dosya, ağ, süreç, ortam
değişkeni ve saat kullanmaz (CI: `check-core-io.sh`). Kaynaklar §6'daki
callback'ten, zaman §7'deki `now_ns`'ten, yapılandırma `ErkConfig`'ten gelir.
Saati okuyan tek yer Erk'in kendi olay döngüsüdür (`erk` crate'i, çekirdeğin
dışında) ve onu da `now_ns` olarak verir.

---

## 2. Tipler

- **İç temsil değişmez:** `erk-dom`'daki `NodeId`, 32 bit indeks ve 32 bit
  nesil (`NonZeroU32`). Nesil her slot yeniden kullanımında artar; tükenen
  slot emekliye ayrılır, bugünkü gibi.
- **Dış temsil (`ErkNodeId`):** opak `uint64_t`. Uygulamaya özel bir anahtarla
  karıştırılır. Amacı **ad alanı ayrımı ve eski id yalıtımıdır**: bir
  uygulamanın id'si başka bir uygulamada ya da yok edilmiş bir uygulamada
  kullanılırsa sessizce başka bir düğüme denk gelmesin, hata koduna düşsün.
  Bir güvenlik önlemi **değildir** (aşağıda):

  ```text
  iç          = index | (generation << 32)
  dış         = iç ^ app_key            (host'a giden)
  iç          = dış ^ app_key           (host'tan gelen)
  app_key     = splitmix64(uygulama_sıra_no) & 0xFFFF_FFFF
  ```

  `uygulama_sıra_no` süreç içinde her `erk_app_create`'te bir artan, hiç
  tekrar etmeyen 64 bitlik bir sayaçtır; anahtar rastgele değil, bu yüzden
  testler belirleyicidir. Anahtar yalnızca indeks yarısını karıştırır: dış
  id'nin üst yarısı nesildir ve nesil hiçbir zaman 0 olmadığı için dış id de
  hiçbir zaman 0 olmaz; 0 her zaman "düğüm yok"tur (`ERK_NODE_NONE`), bir
  düğümün yerine verilirse `ERK_ERR_INVALID_ARGUMENT`.
- **Geçerlilik her zaman arenadan gelir:** çözülen indeks var mı, o slotun
  nesli eşleşiyor mu. Başka bir uygulamanın ya da yok edilmiş bir
  uygulamanın id'si çözülünce neredeyse her zaman var olmayan ya da nesli
  tutmayan bir slota düşer ve `ERK_ERR_STALE_NODE` döner. Yakalama **olasılıksaldır**: yanlış bir id'nin
  geçerli bir çifte denk gelme olasılığı canlı düğüm sayısına ve nesillerin
  dağılımına bağlıdır, sabit bir oran olarak verilmez.
- **Karıştırma ne değildir.** Bir güvenlik sınırı, kimlik doğrulama,
  yetkilendirme ya da sahteciliğe karşı koruma (anti-forgery) **değildir**.
  Anahtar gizli değildir, uygulama sıra numarasından belirleyici olarak
  türetilir; bir id'yi bilen herkes onu kullanabilir. Host ile Erk aynı
  süreçte, aynı güven alanında çalışır: aralarında korunacak bir sınır
  yoktur. Gizli ya da tahmin edilemez bir tutamak gereken bir host (örneğin
  id'leri güvenilmeyen bir tarafa veren) bunu kendi katmanında yapar. Bir
  id'yi "imzalı" ya da "doğrulanmış" sayan her belge ve API yanlıştır.
- Host bu sayıyı yorumlamaz, yalnızca saklar ve geri verir. DevTools id'yi
  anahtarla çözüp indeks ve nesil olarak gösterebilir; bu bir hata ayıklama
  kolaylığıdır, ABI'nin parçası değil.

  **Reddedilen düzen:** `uygulama 8 bit | nesil 24 bit | indeks 32 bit`.
  Nesil 24 bite inince, saniyede 60 kez yeniden çizilen bir listenin
  slotları yaklaşık 78 saatte tükenir ve arena emekliye ayrılan slotlarla
  büyür; uzun süre açık kalan bir masaüstü uygulamasında bu bir sızıntıdır.
  8 bit yalnızca 256 etiket verir; bir pencere gün içinde yüzlerce kez
  açılıp kapanınca etiket yeniden kullanılır ve yok edilmiş bir uygulamanın
  id'leri geri döner (yeni arenada nesiller yine 1'den başlar). Yayımlanmış
  bir bit düzeni de host'u bitleri okumaya davet eder ve opaklığı bozar.
- **Eski id:** silinmiş bir düğümün id'si hiçbir zaman başka bir düğümü
  göstermez; her çağrı `ERK_ERR_STALE_NODE` döner, çökmez. `erk_load_html`
  yeni bir arena kurmaz: eski düğümleri siler (nesilleri artar), böylece
  önceki belgenin id'leri de eskir.
- **Tutamaklar:** `ErkApp*` opaktır; `erk_app_create` ile alınır,
  `erk_app_destroy` ile bırakılır.
- **Sayılar:** durum kodları ve sabitler `int32_t`/`uint32_t`'dir, C `enum`'u
  değil; `enum`'un boyutu derleyiciye bağlıdır.
- **Genişletilebilir yapılar:** host'un doldurduğu ya da Erk'in verdiği her
  yapının ilk alanı `uint32_t struct_size`'dır. Sonraki sürümler sona alan
  ekler; kısa bir yapı eksik alanların varsayılan değeri demektir.

## 3. Dizeler ve bellek

- **Giriş:** `ErkStr { const char *ptr; size_t len; }`, UTF-8, NUL ile
  bitmesi gerekmez. Erk çağrı dönmeden kopyalar; host belleği hemen geri
  alabilir. Geçersiz UTF-8 → `ERK_ERR_INVALID_ARGUMENT`.
- **Çıkış, küçük değerler:** çağıranın tamponu:
  `erk_node_text(app, node, char *buf, size_t cap, size_t *len)`. `*len`
  her zaman gereken uzunluğu verir; tampon küçükse `ERK_ERR_BUFFER_TOO_SMALL`
  döner ve hiçbir şey yarım yazılmaz.
- **Çıkış, büyük değerler** (inspector dökümleri): `ErkString`, Erk'in
  belleği, `erk_string_free` ile serbest bırakılır. İki allocator hiçbir
  zaman karışmaz: Erk'in verdiği bellek yalnızca Erk'e geri verilir.
- **Olay verisi:** `ErkEvent` ve içindeki dizeler yalnızca callback
  süresince geçerlidir.

## 4. İş parçacığı modeli

- `ErkApp`'i oluşturan iş parçacığı **UI iş parçacığıdır**. API'nin tamamı
  yalnızca oradan çağrılır. Başka bir iş parçacığından gelen çağrı hiçbir şey
  yapmadan `ERK_ERR_WRONG_THREAD` döner.
- İki istisna, her iş parçacığından çağrılabilir:
  - `erk_app_post(app, fn, user_data)`: `fn` bir sonraki kareden önce UI iş
    parçacığında çalışır. Arka plan işinden UI'a dönmenin tek yolu.
  - `erk_resource_complete(...)` (§6).
- macOS'ta pencere ana iş parçacığı ister; orada UI iş parçacığı ana iş
  parçacığı olmalıdır.
- **UI iş parçacığının yığını (karar, M3.1).** Layout her iç içelik
  düzeyinde bir kez özyinelenir (bir kısmı Taffy'nin içinde); ayrıştırıcı
  derinliği Chrome gibi 512'de keser. O belge release'de 2–4 MiB, debug'da
  4–8 MiB yığın istiyor; Windows'ta ana iş parçacığının yığını 1 MB. Bu
  yüzden bir karenin stili, layout'u ve display list'i 16 MiB yığınlı
  kare iş parçacığında, UI iş parçacığı beklerken çalışır; belge ve
  kaynakları kare için oraya taşınıp geri gelir. Kare iş parçacığı süreçte
  tektir ve hiç durmaz: Stylo iş parçacığına özel önbelleklerini bilerek
  sızdırıyor, kare başına açılan iş parçacığı her karede ~13 KB bırakırdı
  (M3.5). Belge yine UI iş parçacığının: değişiklikler ve sorgular orada,
  anında. Host'tan büyük bir yığın beklenmez: motorun her işlemi en
  derin belgede 1 MiB yığınla çalışır (test).

## 5. Callback'ler

- **Ne zaman:** her callback UI iş parçacığında, kareler arasında çalışır;
  stil, layout ya da boyama sırasında **asla**.
- **İçinde ne yapılabilir:** değişiklik ve sorgu çağrıları serbesttir.
  Değişiklikler bir sonraki kareden önce uygulanır. `erk_app_run` ve
  `erk_app_destroy` callback içinden çağrılamaz (`ERK_ERR_REENTRANT`).
- **Ömür:** abonelik `user_data` ve isteğe bağlı bir `destroy` alır.
  `destroy` tam bir kez çalışır: `erk_off` çağrılınca, düğüm silinince ya da
  `erk_app_destroy`'da, hangisi önce olursa. Bir callback kendi aboneliğini
  kaldırırsa `destroy` callback döndükten sonra çalışır.
- **Olaylar:** host belirli düğümlere belirli olay türleri için abone olur.
  Dağıtım DOM'un capture, target ve bubble alt kümesidir. Host tanımadığı olay
  türlerini yok saymalıdır; yeni türler eklenebilir.
- **Capture aboneliği (M3.2):** DOM'daki gibi bir abonelik ya capture ya
  bubble aşamasınındır; ikisi de hedefte çağrılır, önce capture olanlar.
  Rust'ta `on` ve `on_capture`; C-ABI'de `erk_on`'un yanına `erk_on_capture`
  eklenir (M3.5, ekleme olduğu için kırıcı değil). Odak ve odak kaybı
  kabarmaz (UI Events), tıklama kabarır. `stop_propagation` olayı bulunduğu
  düğümde bitirir; o düğümdeki öteki abonelikler yine çalışır. Bir
  callback'in kaldırdığı, henüz çağrılmamış abonelik çağrılmaz.

## 6. Kaynaklar

Erk hiçbir dosyayı kendisi okumaz. İçerik bir kaynak istediğinde (CSS `url()`,
`<img src>`, `<link rel=stylesheet>`, ileride `@font-face`), UI iş
parçacığında host'un `ErkResourceFn`'i çağrılır. İstek ve yanıt yapılandırılmıştır:

- **İstek:** bir id, URL ve kaynağın türü (`ERK_RESOURCE_IMAGE`,
  `ERK_RESOURCE_STYLESHEET`, `ERK_RESOURCE_FONT`). Tür, host'un aynı URL'yi
  farklı biçimlerde sunabilmesi ve yanlış türde veriyi baştan reddedebilmesi
  içindir. Rust API'sinde `ResourceRequest { id, url, kind }`.
- **Yanıt:** `erk_resource_complete(app, id, status, mime, data, len)`; Rust
  API'sinde `ResourceResponse { mime, data }`. MIME boş bırakılırsa Erk türü
  içerikten ve istek türünden çıkarır. Bu çağrı callback'in içinden
  (eşzamanlı) ya da sonra başka bir iş parçacığından olabilir. Erk veriyi
  kopyalar.
- Durum `ErkStatus`'tur, HTTP kodu değil: ağ yok, 404 ile 500 arasındaki
  fark host'undur ve Erk için hepsi "kaynak yok" demektir.
- `ERK_ERR_NOT_FOUND` ya da hiç yanıt vermemek kaynağı yok sayar; sayfa onsuz
  çizilir. Rust API'sinde yanıt bir `Responder`'dır: yanıtlanmadan bırakılan
  `Responder` kaynağı yok sayar (M3.2), böylece kare "kaynak bekleniyor"
  durumunda kalmaz.
- Host sağlayıcı yoksa hiçbir kaynak yüklenmez.
- Demo kabuğun sağlayıcısı yalnızca açılan dosyanın dizinini ve `memory://`
  şemasını kabul eder; `file:///etc/passwd` ve kök dışına çıkan yollar
  reddedilir.

### 6.1 Erişilebilirlik bilgisi

Erişilebilirlik bilgisi (rol, etiket, durum) DOM'daki elemanın anlamından ve
özniteliklerinden (`role`, `aria-label`, `aria-*`) okunur; host bunları
sıradan öznitelikler olarak yazar. `erk-dom`'un düğüm yapısına şimdiden
`aria_role` ya da `aria_label` alanı eklenmez: öznitelikler zaten saklanıyor ve
boş alanlar M5'e kadar ölü kod olurdu. AccessKit ağacı M5'te bu özniteliklerden
kurulur.

### 6.2 Fontlar (M1.7)

**Karar: sistem fontlarını host tarar, çekirdek yalnızca bir katalog ve
istediği fontların baytlarını alır.** Font dosyası okumak, işletim sisteminin
font listesini sorgulamak ve kullanıcının fontconfig ayarını okumak G/Ç'dir;
çekirdek bunların hiçbirini yapmaz (§6, check-core-io).

- **Tarama host'ta:** demo kabuk (M3'te `erk`) fontique'in sistem
  taramasıyla platformun kendi kaynağını kullanır: Windows'ta DirectWrite,
  Linux'ta fontconfig (çalışma anında `dlopen` ile; bağlama zamanında C
  bağımlılığı yok, kütüphane yoksa yalnızca gömülü font kalır), macOS'te
  CoreText. Chrome da yedek fontu bu kaynaklardan seçiyor.
- **Katalog veri olarak gelir** (`ToRenderer::Fonts(FontCatalog)`, M3'te
  `ErkConfig`'te ya da ayrı bir çağrıda): ailelerin adları (font
  dosyalarının verdiği adlar), generic ailelerin eşlemesi (`sans-serif`,
  `serif`, `monospace`, `cursive`, `fantasy`, `system-ui`, `emoji`) ve yazı
  sistemine (ISO 15924, isteğe bağlı dil) göre yedek aile listeleri.
  **Yüzler katalogda yok:** bir ailenin yüzlerini listelemek dosyalarını
  açmak demek; Windows'ta 200 ailenin 399 yüzü 1,3 s sürdü, katalog 12 ms.
- **Baytlar kaynak API'siyle gelir:** çekirdek belgenin kullandığı her yüzü
  `ResourceKind::Font` ile `font:<aile>?weight=<n>&style=<normal|italic>`
  olarak ister (ailede `%`, `?`, `&`, `#` yüzde kodlu); host o ailenin bu
  ağırlık ve stile en yakın yüzünü seçer ve dosyasını verir. MIME
  `font/ttf`, `font/otf`, `font/collection` ya da boş olmalı; font olmayan
  baytları fontique kaydetmez, metin yedek fontla çizilir. Gelen fontlar
  belge değişse de saklanır; bir yüz ikinci kez istenmez.
- **Gömülü Noto Sans her zaman son yedektir.** Katalog yoksa (testler,
  `render_html`) her aile ona çözülür; bu yüzden altın görüntüler ve Chrome
  referans testleri makineden bağımsız kalır.
- **Erk makinenin dilini okumaz:** yedek seçimi metnin `lang`'ından gelir
  (Japonca ve Çince Han karakterleri farklı fontlarla çizilir); dili
  olmayan metin için host'un kataloğundaki dilsiz liste kullanılır.

Reddedilen yol: fontique'in sistem taramasını çekirdekte açmak. Çekirdek
dosya okur hâle gelirdi ve aynı sayfa her makinede başka ölçülürdü; testler
deterministik kalmazdı. Reddedilen ikinci yol: font dizinlerini saf Rust ile
taramak. Yeni bağımlılık getirmezdi, ama yedek listeleri bizim tablolarımız
olurdu ve fontconfig ayarları ile platformun yedek kuralları yok sayılırdı.

## 7. Zaman ve olay döngüsü

- **Erk'in döngüsü** (M3): `erk_app_run(app)` pencereyi açar ve pencere
  kapanana kadar döner. Döngü monoton saati okur ve her kareye `now_ns`
  olarak verir.
- **Host'un döngüsü** (M3 sonrası, burada tanımlanır): host kendi döngüsünü
  çalıştırır. Kendi penceresini raw-window-handle ile verir, girdiyi
  `erk_app_input` ile iletir ve her turda `erk_app_tick(app, now_ns)` çağırır.
- Çekirdek saati hiç okumaz. İmleç yanıp sönmesi (M5), geçişler ve
  animasyonlar (M9) yalnızca `now_ns`'e bağlıdır; testler zamanı kendileri
  sürer.

## 8. Hata modeli

- Her fonksiyon (`void` dönenler hariç) bir `ErkStatus` döner; değerler
  çıkış parametreleriyle gelir.
- **Panik sınırı geçmez.** Her `extern "C"` fonksiyonun gövdesi
  `catch_unwind` içindedir. Bir panik `ERK_ERR_PANIC` döner ve uygulamayı
  **zehirler**: sonraki her çağrı `ERK_ERR_POISONED` döner, yalnızca
  `erk_app_destroy` çalışır. Zehirli bir belge üzerinde devam etmek,
  tutarsız bir ağacı çizmek demektir.
- Rust API'sinde host'un kendi closure'undan çıkan panik, temizlikten sonra
  `run`'ı çağırana taşınır (`resume_unwind`).

## 8.1 Denetim (geliştirici araçları)

Geliştirici araçları (M7) motora ayrı bir kapıdan değil, bu sözleşmenin salt
okunur sorgularıyla bakar; aynı API'yi host da kullanabilir.

- **Sorgular** (M3): düğüm ağacı (`erk_node_parent`, çocuklar), etiket ve
  öznitelikler, hesaplanmış stil (özellik adı ve değeri, metin olarak),
  kutu modeli (konum, boyut, margin, border, padding; CSS pikseli). Hepsi
  UI iş parçacığından, eski id'de `ERK_ERR_STALE_NODE`.
  **M3.4'te koda inen biçim:** ağaç için taslaktaki `parent` ve
  `child_at`'e ek olarak çocuk sayısı ve düğümün türü (belge, eleman,
  metin, yorum, diğer); etiket ve öznitelikler ayrı sorgular. Kutu ve
  hesaplanmış stil son karenindir: ilk kareden önce, kutusuz düğümde
  (metin, satır içi eleman, gösterilmeyen) `ERK_ERR_NOT_FOUND`. Hesaplanmış
  stil Erk'in kullandığı longhand'lerin ad sırasıyla `ad: değer;`
  satırları; Stylo bütün longhand'leri dolaşmanın yolunu vermiyor ve
  kalanlar Erk'in yok saydığı değerler olurdu. C-ABI'deki karşılıkları
  (`erk_node_child_count`, `erk_node_kind`, `erk_node_tag`,
  `erk_node_attributes`) M3.5'te, taslağa ekleme olarak gelir.
- **Seçme ve vurgu** (M2): `erk_inspect_at(app, x, y, &node)` hit-test'in
  sonucunu döndürür; `erk_highlight(app, node)` seçili düğümün kutularını bir
  kaplamayla çizer. Kaplama display list'e eklenir, belgeye değil: DOM'da ve
  hesaplanmış stillerde iz bırakmaz.
- **Aşama süreleri** (M3): kare başına stil, layout, display list ve raster
  süreleri. Çekirdek saat okumadığı için (§1.3) süreleri aşamaları sırayla
  çağıran `erk` crate'i ölçer; çekirdeğe saat girmez.
- **Log** (M3): `ErkLogFn` Erk'in kendi uyarılarını da taşır (engellenen ya
  da yüklenemeyen kaynak); DevTools'un Console paneli bunları ve host'un
  loglarını gösterir.
- **Canlı düzenleme** (M5): stil değişikliği sıradan bir değişikliktir
  (`erk_node_set_attr(node, "style", ...)` ya da stil sayfası güncellemesi),
  artımlı yeniden stille görünür. DevTools'a özel bir yazma yolu yoktur.
- **Taşıma:** DevTools önce aynı süreçte ikinci bir pencere olarak çalışır.
  Ayrı süreçte bir DevTools ancak host açıkça etkinleştirirse ve yerel bir
  kanal üzerinden bağlanır; motor varsayılan olarak hiçbir portu dinlemez.

## 8.2 Surface (M10, yalnızca yön)

Host'un belgenin içindeki bir bölgeye kendi GPU çizimini yapması. Bugün
sabitlenen tek kural: **bir pencere yüzeyinin tek çizicisi vardır, Erk.**
Host aynı yüzeye kendi başına yazmaz; Erk kareyi kurarken surface bölgesi için
ya host'un çizim callback'ini (paylaşılan wgpu cihazı ve komut kodlayıcı)
çağırır ya da host'un dokusunu bölgeye yerleştirir. Bölgeyi host bir düğümle
kaydeder (`erk_surface_create(node)`); konumu, kaydırması ve kırpması
layout'tan gelir.

**M2.5'in ölçümünden sonra yön: doku.** vello_hybrid bir kareyi tek bir
`render` çağrısıyla, tek geçişte çiziyor; araya host'un çizimini sokacak bir
yer yok, callback ancak kareyi bölüp birkaç geçişe ayırmakla olur. Buna karşın
dışarıdan bir dokuyu bir bölgeye yerleştirmeyi zaten biliyor
(`TextureBindings`, `draw_texture_rects`): host kendi dokusuna, Erk'in
cihazında, kendi zamanında çizer; Erk onu display list'teki yerinde,
kırpması ve opaklığıyla, sırası bozulmadan çizer. Kesin karar M10'da, M3'ün
API'si oturunca; bu yön onu bağlamaz.

## 9. Sürümleme

- `erk_abi_version()` → `(major << 16) | minor`. Bir ana sürüm içinde
  yalnızca ekleme yapılır: yeni fonksiyon, yeni sabit, yapıların sonuna alan.
  Kaldırma ya da imza değişikliği ana sürümü artırır.
- 1.0'a kadar (M8) ana sürüm 0'dır ve her küçük sürüm kırıcı olabilir; bu
  belge kırılmaları yazar.

---

## 10. `erk.h`

M3.5'ten beri başlık elle yazılmıyor: `crates/erk-ffi`'den cbindgen ile
üretilir ve [`include/erk.h`](../../include/erk.h)'de durur; üretilenin
depodakiyle aynı olduğunu bir test denetler. M0.5'in elle yazılmış taslağı
(v0.1) git geçmişinde. v0.2'nin taslaktan farkları, her biri ekleme ya da
gerekçeli bir daraltma:

- **Eklenenler:** `ERK_ERR_PLATFORM` (pencere ya da olay döngüsü
  kurulamadı); `ERK_LOG_*` düzeyleri; `ErkConfig`'in sonuna `flags`
  (`ERK_APP_HEADLESS`, `ERK_APP_EMBEDDED_FONTS`, `ERK_APP_CPU`); ekransız
  uygulama için `erk_app_tick`, `erk_app_input` (`ErkInput`, `ERK_INPUT_*`,
  `ERK_BUTTON_*`, `ERK_KEY_*`, `ERK_MOD_*`) ve `erk_app_frame` (`ErkFrame`);
  `erk_on_capture` (§5); denetim için `erk_node_child_count`,
  `erk_node_kind` (`ERK_NODE_*`), `erk_node_tag`,
  `erk_node_attribute_count`, `erk_node_attribute_at` (§8.1).
- **M4'e kalanlar** (M3 planı, karar 1): `erk_node_create`,
  `erk_text_create`, `erk_node_append`, `erk_node_insert_before`,
  `erk_node_remove`, `erk_node_set_attr`, `erk_node_remove_attr`. Motorda
  karşılıkları `Mutation` API'siyle gelir; eklemek kırıcı değil.
- **Daraltılanlar:** `erk_on`, Erk'in henüz üretmediği olay türlerinde
  (girdi, değişiklik, gönderim, tuş: formlarla M5) `ERK_ERR_INVALID_ARGUMENT`
  döner. Başarısız bir `erk_on` `user_data`'yı almaz, `destroy`'u çağırmaz:
  sahiplik host'ta kalır.
- **Kütüphanenin adı:** workspace'te `erk` crate'i ve `erk` ikilisi olduğu
  için (Windows'ta aynı adlı `.pdb` birbirini ezer) derleme çıktısı
  `erk_ffi.dll`/`liberk_ffi.so`/`liberk_ffi.dylib`; dağıtılan kütüphanenin
  `erk` adını alması paketlemenin işi (M8).

## 11. Kural → muhafız takvimi

Muhafız ilkesi gereği her kuralın muhafızı koruduğu kodla aynı PR'da gelir.
Bu tablo hangi kuralın ne zaman ve nasıl zorlanacağını söyler.

| Kural | Zorlama | Taş |
|---|---|---|
| Çekirdek G/Ç, ortam ve saat kullanmaz (§1.3) | `check-core-io.sh` | Bugün |
| UI ↔ raster sınırı düz veri (§1.1) | `check-renderer-surface.sh`: mesajlar ve M3.0'dan beri display list ile tablo güncellemeleri (`list.rs`) yalnızca prelude tipleri ve kendi tipleri; display list tipleri başka dosyada tanımlanamaz | Bugün, M3.0 |
| `erk-renderer` pencere katmanını bilmez | CI: `cargo tree -p erk-renderer` çıktısında `winit`/`softbuffer` yok | M1 |
| `unsafe` yalnızca `erk-style` ve `erk-ffi`'de | Lint devralma istisna listesi (CI); `check-ffi.sh`: `erk-ffi`'de her `unsafe` izni (tek başına, başka lint'lerle ya da `expect` olarak) `// SAFETY:` gerekçeli | M3.5 |
| `erk.h` üretilir ve güncel | `erk-ffi`'nin testi cbindgen ile üretip `include/erk.h`'yle karşılaştırır; C örneği üç işletim sisteminde derlenip çalışır | M3.5 |
| Panik sınırı geçmez, zehirlenme (§8) | `check-ffi.sh`: her dışa açılan işlev `guard`/`guard_free`/`subscribe`'dan geçer; enjekte edilen panikle test (`PANIC`, sonra `POISONED`, `destroy` çalışır) | M3.5 |
| Yanlış iş parçacığı (§4) | Başka iş parçacığından çağıran test (hiçbir şey değişmiyor); `erk_app_post` ve `erk_resource_complete` başka iş parçacığından | M3.5 |
| Dizeler kopyalanır, tampon yarım yazılmaz (§3) | C örneği: girdiyi çağrıdan hemen sonra eziyor, `BUFFER_TOO_SMALL`'da tamponun değişmediğini denetliyor; Linux'ta AddressSanitizer ile | M3.5 |
| `destroy` tam bir kez (§5) | Sayaçlı test: `erk_off`, düğüm silme ve `erk_app_destroy` yollarının üçü, başarısız `erk_on`'da hiç | M3.5, M4 |
| Başka ya da yok edilmiş bir uygulamanın id'si hiçbir düğümü göstermez (§2) | İki `ErkApp`'te aynı sırayla oluşturulan düğümler: birinin id'leri ötekinde `ERK_ERR_STALE_NODE`; yok edilip yeniden oluşturulan bir uygulamada eski id'ler de; dış id hiçbir zaman 0 değil (özellik testi) | M3 |
| Eski id hiçbir düğümü göstermez (§2) | Birim testi; `Mutation` fuzz'ı; `erk_load_html` sonrası eski id testi | M3, M4 |
| Yanlış türde kaynak reddedilir (§6) | Görüntü isteğine stil sayfası verisiyle yanıt veren test | M1 (görüntüler gelince) |
| Kaynak yalnızca callback'ten (§6) | `url("file:///...")` içeren sayfada hiçbir dosyanın okunmadığını ve sağlayıcının çağrıldığını doğrulayan test | M1 (görüntüler gelince) |
| Vurgu kaplaması belgeye girmez (§8.1) | Vurgu açıkken ve kapalıyken DOM dökümü ve hesaplanmış stiller aynı; display list'te yalnızca kaplama öğesi farklı | M2 |
| Aşama süreleri çekirdeğe saat sokmadan ölçülür (§8.1) | `check-core-io.sh` zaten `Instant::now`'ı çekirdekte yasaklıyor; `erk_last_frame_timings` testi | M3 |
| Yapılar genişletilebilir, sürüm (§2, §9) | Kısa `ErkConfig` (alanları okunmuyor) ve kısa `ErkBox` (alanları yazılmıyor) testi; `erk_abi_version` testi | M3.5 |
