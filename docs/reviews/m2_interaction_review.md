# 📝 Erk Engine (M2: Interaction & Text Layout) - Kapsamlı Kod İnceleme Raporu

**İncelenen Commit Aralığı:** `a5d9c9f` ... `35d9056` (M2.0 - M2.6)
**Ana Odak:** Kalıcı Document mimarisi, Pointer & Hit-Testing, Etkileşim Durumları (:hover, :focus), Klavye/Focus, Overflow & Scroll, İlk DOM Mutasyonları (Counter Demosu) ve Gelişmiş Metin Yerleşimi (<br>, white-space).

M2 aşamasıyla birlikte Erk Engine statik bir çizim motorundan tam teşekküllü, etkileşimli (interactive) bir UI motoruna evrilmiş durumda. Tüm commit'leri mimari, kod kalitesi ve CSS standartlarına uyum açısından inceledim.

---

### 🌟 Öne Çıkan Başarılar ve Mimari Kararlar

1. **Kalıcı Document Yapısı (M2.0 - `page.rs`):**
   * Her frame'de HTML'i sıfırdan parse etmek yerine, `Load` anında bir kez parse edip document ağacını canlı tutmak (`Page` struct'ı üzerinden) çok doğru bir temel. Parse maliyetinin ihmal edilebilir olmasına rağmen, `:hover`, scroll pozisyonları ve host tarafından tetiklenecek metin mutasyonlarının saklanabilmesi için bu mimari zorunluydu.
   * Chrome'un `Range.getClientRects()` çıktılarıyla metin çizgilerinin (`{page}.text.txt`) piksel düzeyinde kıyaslanması, text yerleşimlerindeki gizli görsel hataların erkenden yakalanmasını sağlamış.

2. **Paint-Order Hit-Testing Mimarisi (M2.1 - `display.rs`, `page.rs`):**
   * Hit-testing için ayrı bir geometri ağacı (spatial tree) kurmak yerine, boyama sırasına (`paint order`) göre unpainted `Hit` display item'ları eklenmesi muhteşem bir sadelik ve doğruluk sağlamış. Stacking context ve `z-index` kuralları doğrudan çizim katmanından miras alınıyor; en üstteki tıklanan element naturally tespit ediliyor.
   * `pointer-events: none` ve `visibility: hidden` durumlarında hit oluşturulmaması standarda tam uygun.

3. **Gereksiz Render'ı Önleyen Reaktivite (M2.2 - `erk-style`):**
   * `Styles::react_to` fonksiyonu ile Stylo'nun kural bağımlılıkları kontrol ediliyor. Eğer sayfada hiçbir `:hover` kuralı yoksa, fare hareket ettikçe sayfa tekrar tekrar boyanmıyor (repaint maliyeti sıfıra iniyor). Bu optimizasyon performans açısından kritik.
   * `tabindex` sıralaması ve odak yönetimi (pozitif olanlar önce, negatif olanlar atlanarak) erişilebilirlik standartlarını karşılıyor.

4. **CSS Overflow ve Scope Tabanlı Kırpma (M2.3 - `scroll.rs`):**
   * `scroll.rs` içerisindeki scope yönetimi CSS spesifikasyonuna harfiyen uyuyor:
     * Kök elementin (`html` veya `body`) overflow'unun doğrudan viewport'a devredilmesi (propagation).
     * `absolute` konumlanan kutuların, kapsayıcı bloklarının dışındaki kırpma (clip) atalarından kaçabilmesi (`escape clips`).
     * `fixed` elemanların viewport dahil tüm clip'lerden muaf olması.
   * Bu kurallar çoğu web motorunun bile zorlandığı detaylardır, Erk Engine'de tertemiz modellenmiş.

5. **İlk Etkileşimli Demo ve Güvenli ID Yönetimi (M2.4 - `counter.rs`, `arena.rs`):**
   * `Counter` demosu ile event bubbling, querySelector ve `set_text` döngüsünün eksiksiz çalıştığı kanıtlanmış.
   * Yeni bir sayfa yüklendiğinde eski node ID'lerinin yeni sayfadaki node'lar ile çakışmaması (StaleNode garantisi - p1-contract §2) için `arena`'nın korunarak ID aralığının ilerletilmesi çok dikkatli bir detay.

6. **Parley Hatası ve Inline Kenar Çözümü (M2.6 - `text.rs`):**
   * Parley'in satır sonlarında inline kutuları (`border`/`padding`) metninden ayırıp tek başına bırakma veya satır başına tek border koyma hatasının (`border-fragmentation`), satırların Erk tarafından daraltılarak tekrar kırılmasıyla çözülmesi zekice bir workaround.
   * `<br>` etiketinin satır sonu davranışları ve `white-space: nowrap | pre | pre-wrap | pre-line` modları CSS Text standartlarına tam oturtulmuş.

---

### ⚠️ Dikkat Edilmesi Gereken Ufak Noktalar (Gelecek İçin)

1. **Rounded Corners & Overflow Clipping:**
   * M2.3 notlarında da belirtildiği gibi, `overflow: hidden` olan bir kutunun köşe yuvarlatması (`border-radius`) varsa, içerideki çocuk elemanların bu eğimli köşelere göre değil, henüz düz dikdörtgen padding kutusuna göre kırpıldığı görülüyor. M2/M3 ilerleyişinde clip path'lere `rounded_rect` eklenmesi görsel bütünlüğü tamamlayacaktır.
2. **Sağdan Sola (RTL) Paragraf Yönü:**
   * M1'den beri bilinen "RTL paragraf yönünün CSS `direction` yerine ilk harften alınması" durumu devam ediyor. İleride Parley güncellemesi veya bidi-reorder adımı ile ele alınabilir.

---

### 💬 Sonuç

M2 planlanan hedeflerine muazzam bir hız ve kaliteyle ulaşmış. Kod tabanı genişlerken mimari disiplinden (I/O-free core, P1 kontratı, test korumaları, mutasyon testleri) zerre ödün verilmemiş. Özellikle `scroll.rs` ve `page.rs` modülleri endüstri standardı bir titizlikle yazılmış.

Main branch'teki son durum kusursuz görünüyor!
