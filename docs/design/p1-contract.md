# Erk Engine — P1: Gömme sözleşmesi (M0.5)

- **Tarih:** 2026-09-30
- **Durum:** ABI v0.1, taslak. M3'te koda dönüşür; o zamana kadar değişiklik
  bu belgede, gerekçesiyle yapılır. M1–M5 boyunca öğrenilenlerle v0.2, v0.3
  diye ilerler; 1.0'a (M8) kadar kırıcı değişiklik hakkı saklıdır.
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
  düz veri olmalı. Bugün glyph run'lar `FontData` taşıyor (içinde paylaşılan
  bir blob); M3'te raster tarafında bir font tablosu ve display list'te
  `FontId` olur.
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
  karıştırılır:

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
  nesli eşleşiyor mu. Karıştırma bir güvenlik sınırı ya da kimlik doğrulama
  değildir; başka bir uygulamanın ya da yok edilmiş bir uygulamanın id'sini
  anlamsızlaştıran bir ad alanı ayrımıdır. Böyle bir id çözülünce neredeyse
  her zaman var olmayan ya da nesli tutmayan bir slota düşer ve
  `ERK_ERR_STALE_NODE` döner. Yakalama **olasılıksaldır**: yanlış bir id'nin
  geçerli bir çifte denk gelme olasılığı canlı düğüm sayısına ve nesillerin
  dağılımına bağlıdır, sabit bir oran olarak verilmez.
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
  çizilir.
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
çağırır ya da host'un dokusunu bölgeye yerleştirir. Hangisi olacağı M2'nin GPU
yolu ve M3'ün API'si oturduktan sonra ölçülerek seçilir. Bölgeyi host bir
düğümle kaydeder (`erk_surface_create(node)`); konumu, kaydırması ve kırpması
layout'tan gelir.

## 9. Sürümleme

- `erk_abi_version()` → `(major << 16) | minor`. Bir ana sürüm içinde
  yalnızca ekleme yapılır: yeni fonksiyon, yeni sabit, yapıların sonuna alan.
  Kaldırma ya da imza değişikliği ana sürümü artırır.
- 1.0'a kadar (M8) ana sürüm 0'dır ve her küçük sürüm kırıcı olabilir; bu
  belge kırılmaları yazar.

---

## 10. `erk.h` taslağı

```c
/* erk.h: draft of the Erk embedding ABI (M0.5). Written by hand; from M3 it
 * is generated by cbindgen and CI checks that the committed copy is current. */
#include <stddef.h>
#include <stdint.h>

/* ---- Versions, status codes -------------------------------------------- */

uint32_t erk_abi_version(void);            /* (major << 16) | minor */

typedef int32_t ErkStatus;
#define ERK_OK                    0
#define ERK_ERR_INVALID_ARGUMENT  1        /* null pointer, bad UTF-8, bad value */
#define ERK_ERR_STALE_NODE        2        /* the node was removed */
#define ERK_ERR_WRONG_THREAD      3        /* called from a thread other than the UI thread */
#define ERK_ERR_BUFFER_TOO_SMALL  4        /* *len holds the size needed */
#define ERK_ERR_NOT_FOUND         5
#define ERK_ERR_REENTRANT         6        /* not allowed inside a callback */
#define ERK_ERR_PANIC             7        /* the call panicked; the app is now poisoned */
#define ERK_ERR_POISONED          8        /* an earlier call panicked; only destroy works */

/* ---- Basic types ------------------------------------------------------- */

typedef struct ErkApp ErkApp;              /* opaque */
typedef uint64_t ErkNodeId;                /* opaque, scrambled per app; 0 is no node */
#define ERK_NODE_NONE ((ErkNodeId)0)

typedef struct ErkStr {                    /* UTF-8 in; Erk copies before returning */
  const char *ptr;
  size_t len;
} ErkStr;

typedef struct ErkString {                 /* owned by Erk */
  char *ptr;
  size_t len;
} ErkString;
void erk_string_free(ErkString s);

/* ---- Callbacks --------------------------------------------------------- */

typedef void (*ErkDestroyFn)(void *user_data);
typedef void (*ErkPostFn)(void *user_data, ErkApp *app);

#define ERK_RESOURCE_IMAGE      1
#define ERK_RESOURCE_STYLESHEET 2
#define ERK_RESOURCE_FONT       3

/* Content asks for a resource (CSS url(), <img>, <link>). Answer now or later
 * with erk_resource_complete; not answering means the page renders without it. */
typedef void (*ErkResourceFn)(void *user_data, ErkApp *app,
                              uint64_t request, uint32_t kind, ErkStr url);

typedef void (*ErkLogFn)(void *user_data, uint32_t level, ErkStr message);

/* ---- Application ------------------------------------------------------- */

typedef struct ErkConfig {
  uint32_t struct_size;                    /* sizeof(ErkConfig) */
  uint32_t width, height;                  /* logical pixels */
  float scale;                             /* device pixels per CSS pixel; 0 = from the window */
  ErkStr title;
  ErkResourceFn resource;                  /* may be NULL: no resources load */
  void *resource_user_data;
  ErkLogFn log;                            /* may be NULL */
  void *log_user_data;
  uint32_t log_level;
} ErkConfig;

ErkStatus erk_app_create(const ErkConfig *config, ErkApp **out);
ErkStatus erk_app_destroy(ErkApp *app);    /* runs every pending destroy callback once */
ErkStatus erk_app_run(ErkApp *app);        /* Erk's event loop; returns when the window closes */

/* Thread-safe. */
ErkStatus erk_app_post(ErkApp *app, ErkPostFn fn, void *user_data, ErkDestroyFn destroy);
ErkStatus erk_resource_complete(ErkApp *app, uint64_t request, ErkStatus status,
                                ErkStr mime,               /* may be empty: sniffed */
                                const uint8_t *data, size_t len);

/* Host-driven loop (defined here, implemented after M3). */
ErkStatus erk_app_tick(ErkApp *app, uint64_t now_ns);

/* ---- Document ---------------------------------------------------------- */

ErkStatus erk_load_html(ErkApp *app, ErkStr html);            /* old ids become stale */
ErkStatus erk_document_root(ErkApp *app, ErkNodeId *out);
ErkStatus erk_query(ErkApp *app, ErkNodeId scope, ErkStr selector, ErkNodeId *out);

ErkStatus erk_node_create(ErkApp *app, ErkStr tag, ErkNodeId *out);
ErkStatus erk_text_create(ErkApp *app, ErkStr text, ErkNodeId *out);
ErkStatus erk_node_append(ErkApp *app, ErkNodeId parent, ErkNodeId child);
ErkStatus erk_node_insert_before(ErkApp *app, ErkNodeId parent,
                                 ErkNodeId child, ErkNodeId before);
ErkStatus erk_node_remove(ErkApp *app, ErkNodeId node);       /* the subtree's ids become stale */
ErkStatus erk_node_set_text(ErkApp *app, ErkNodeId node, ErkStr text);
ErkStatus erk_node_set_attr(ErkApp *app, ErkNodeId node, ErkStr name, ErkStr value);
ErkStatus erk_node_remove_attr(ErkApp *app, ErkNodeId node, ErkStr name);
ErkStatus erk_node_text(ErkApp *app, ErkNodeId node,
                        char *buf, size_t cap, size_t *len);  /* *len: bytes needed */

/* ---- Events ------------------------------------------------------------ */

#define ERK_EVENT_CLICK    1
#define ERK_EVENT_INPUT    2
#define ERK_EVENT_CHANGE   3
#define ERK_EVENT_SUBMIT   4
#define ERK_EVENT_KEY_DOWN 5
#define ERK_EVENT_KEY_UP   6
#define ERK_EVENT_FOCUS    7
#define ERK_EVENT_BLUR     8
/* New kinds may be added; ignore kinds you do not know. */

#define ERK_PHASE_CAPTURE 1
#define ERK_PHASE_TARGET  2
#define ERK_PHASE_BUBBLE  3

typedef struct ErkEvent {                  /* valid only during the callback */
  uint32_t struct_size;
  uint32_t kind;
  uint32_t phase;
  ErkNodeId target;
  ErkNodeId current_target;
  double x, y;                             /* logical pixels, pointer events */
  uint32_t modifiers;
  ErkStr text;                             /* input events */
} ErkEvent;

typedef void (*ErkEventFn)(void *user_data, ErkApp *app, const ErkEvent *event);
typedef uint64_t ErkSubscription;

ErkStatus erk_on(ErkApp *app, ErkNodeId node, uint32_t kind,
                 ErkEventFn fn, void *user_data, ErkDestroyFn destroy,
                 ErkSubscription *out);
ErkStatus erk_off(ErkApp *app, ErkSubscription subscription);
ErkStatus erk_event_stop_propagation(ErkApp *app);            /* inside an event callback */

/* ---- Inspection (read-only; used by the developer tools) --------------- */

typedef struct ErkBox {                    /* CSS pixels, relative to the viewport */
  uint32_t struct_size;
  float x, y, width, height;               /* border box */
  float margin[4], border[4], padding[4];  /* top, right, bottom, left */
} ErkBox;

ErkStatus erk_node_parent(ErkApp *app, ErkNodeId node, ErkNodeId *out);
ErkStatus erk_node_child_at(ErkApp *app, ErkNodeId node, size_t index, ErkNodeId *out);
ErkStatus erk_node_box(ErkApp *app, ErkNodeId node, ErkBox *out);  /* ERK_ERR_NOT_FOUND: no box */
ErkStatus erk_node_computed_style(ErkApp *app, ErkNodeId node,
                                  ErkString *out);                 /* "name: value;" lines */
ErkStatus erk_inspect_at(ErkApp *app, float x, float y, ErkNodeId *out);
ErkStatus erk_highlight(ErkApp *app, ErkNodeId node);              /* ERK_NODE_NONE clears */

typedef struct ErkFrameTimings {           /* measured by the embedding layer, not the core */
  uint32_t struct_size;
  uint64_t frame;
  uint64_t style_ns, layout_ns, display_list_ns, raster_ns;
} ErkFrameTimings;
ErkStatus erk_last_frame_timings(ErkApp *app, ErkFrameTimings *out);
```

---

## 11. Kural → muhafız takvimi

Muhafız ilkesi gereği her kuralın muhafızı koruduğu kodla aynı PR'da gelir.
Bu tablo hangi kuralın ne zaman ve nasıl zorlanacağını söyler.

| Kural | Zorlama | Taş |
|---|---|---|
| Çekirdek G/Ç, ortam ve saat kullanmaz (§1.3) | `check-core-io.sh` | Bugün |
| UI ↔ raster sınırı düz veri (§1.1) | Bugün `check-renderer-surface.sh`; M3'te display list ve font tablosu sınırına göre yeniden yazılır | Bugün, M3 |
| `erk-renderer` pencere katmanını bilmez | CI: `cargo tree -p erk-renderer` çıktısında `winit`/`softbuffer` yok | M1 |
| `unsafe` yalnızca `erk-style` ve `erk-ffi`'de | Lint devralma istisna listesi | M3 |
| `erk.h` üretilir ve güncel | cbindgen ile yeniden üretip depodakiyle karşılaştırma | M3 |
| Panik sınırı geçmez, zehirlenme (§8) | Her `extern "C"` gövdesi ortak bir koruma sarmalayıcısından geçer (CI denetler); enjekte edilen panikle test | M3 |
| Yanlış iş parçacığı (§4) | Başka iş parçacığından çağıran test; `erk_app_post` testi | M3 |
| Dizeler kopyalanır, tampon yarım yazılmaz (§3) | Girdiyi çağrıdan hemen sonra ezen test; `BUFFER_TOO_SMALL` testi; C örneği Linux'ta AddressSanitizer ile | M3 |
| `destroy` tam bir kez (§5) | Sayaçlı test: `erk_off`, düğüm silme ve `erk_app_destroy` yollarının üçü | M3, M4 |
| Başka ya da yok edilmiş bir uygulamanın id'si hiçbir düğümü göstermez (§2) | İki `ErkApp`'te aynı sırayla oluşturulan düğümler: birinin id'leri ötekinde `ERK_ERR_STALE_NODE`; yok edilip yeniden oluşturulan bir uygulamada eski id'ler de; dış id hiçbir zaman 0 değil (özellik testi) | M3 |
| Eski id hiçbir düğümü göstermez (§2) | Birim testi; `Mutation` fuzz'ı; `erk_load_html` sonrası eski id testi | M3, M4 |
| Yanlış türde kaynak reddedilir (§6) | Görüntü isteğine stil sayfası verisiyle yanıt veren test | M1 (görüntüler gelince) |
| Kaynak yalnızca callback'ten (§6) | `url("file:///...")` içeren sayfada hiçbir dosyanın okunmadığını ve sağlayıcının çağrıldığını doğrulayan test | M1 (görüntüler gelince) |
| Vurgu kaplaması belgeye girmez (§8.1) | Vurgu açıkken ve kapalıyken DOM dökümü ve hesaplanmış stiller aynı; display list'te yalnızca kaplama öğesi farklı | M2 |
| Aşama süreleri çekirdeğe saat sokmadan ölçülür (§8.1) | `check-core-io.sh` zaten `Instant::now`'ı çekirdekte yasaklıyor; `erk_last_frame_timings` testi | M3 |
| Yapılar genişletilebilir, sürüm (§2, §9) | `struct_size`'ı kısa bir yapıyla çağıran test; `erk_abi_version` testi | M3 |
