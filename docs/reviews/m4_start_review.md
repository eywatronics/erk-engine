# 📝 Erk Engine (M3 Kapanışı & M4.0 Başlangıcı) - Kod İnceleme Raporu

**İncelenen Commitler:**
* `d556b98` - M3.5: the C ABI, erk.h generated, one guard for every call (#45)
* `549a001` - M3.6: acceptance, a Rust host beside the C one; M3 done (#46)
* `7d93f4f` - M4: plan; M2 and M3 measured side by side (#47)
* `2ec4c68` - M4.0: build the document from the host (#48)

---

### 🌟 1. M3 Kapanışı ve Kritik Bellek Düzeltmesi (M3.5 - M3.6)

1. **C ABI ve `erk.h` (M3.5):**
   * `cbindgen` kullanılarak `include/erk.h` başlık dosyası sıfır sapmayla üretildi.
   * C'den gelen tüm çağrılar tek bir muhafız (`guard`) üzerinden denetleniyor: yanlış thread'den çağrılırsa `WRONG_THREAD`, panik olursa C'yi çökertmeyip uygulamayı zehirleyerek `PANIC` dönüyor.
2. **Kritik Düzeltme (Stylo Thread-Local Bellek Sızıntısı):**
   * C örneği AddressSanitizer altında koşulurken, Stylo'nun thread-local depolamasında (bloom filtreleri ve stil paylaşım önbellekleri) kasıtlı olarak temizlemediği statik bellek tahsisleri tespit edildi.
   * M3.1'de kare başına açılan `erk-frame` yardımcı thread'i her karede ~13 KB bellek sızdırıyordu (60 FPS'te saatte ~3 GB sızıntı!).
   * **Çözüm:** Kare hazırlama işlemi "kare başına bir thread" yerine "tüm süreç için tek bir kalıcı thread" modeline geçirildi. Stylo önbellekleri bir kez oluşturulup tekrar kullanıldı; bellek sızıntısı tamamen sıfırlandı ve 0.19 ms'lik thread açılış gecikmesi ortadan kalktı!
3. **M3 Kabulü (M3.6):**
   * Hem C (`hello.c`) hem Rust (`hello.rs`) örnekleri Linux, Windows ve macOS üzerinde CI'da başarıyla çalıştı ve M3 "Bitti" olarak işaretlendi.

---

### 🚀 2. M4 Başlangıcı: Etkileşimli DOM (M4.0)

M4.0 ile motor salt-okunur olmaktan çıkıp, host uygulamanın belgeyi dinamik olarak inşa edebildiği (create/insert/remove/attr) bir aşamaya geçti:

1. **W3C Standartlarına Uygun Ön-Ekleme Denetimi (`insert`):**
   * `crates/erk-dom/src/document.rs` içerisindeki hiyerarşi denetimleri; bir düğümün kendi içine veya kendi soyundan gelen bir elemana eklenmesini, `Document` düğümünün taşınmasını veya içine doğrudan metin (`Text`) eklenmesini başarıyla engelliyor (`MutationError::Hierarchy`).
   * `<template>` elemanları oluşturulurken içerikleri otomatik olarak `DocumentFragment` olarak ayrıştırılıyor.
2. **Kalıntısız Silme ve Bellek Koruması (`remove` / `forget_gone_nodes`):**
   * Bir alt ağaç silindiğinde o ağaçtaki tüm olay abonelikleri tek seferlik `destroy` çağrısıyla sonlandırılıyor.
   * `forget_gone_nodes` fonksiyonu silinen elemanlara ait scroll pozisyonlarını temizliyor. Bu sayede eleman oluşturup silen dinamik sayfalarda bellek sızıntısı yaşanmıyor (M4.2'deki 10.000 döngü testi için kritik zemin).
3. **C ABI ve Kolaylık Fonksiyonları:**
   * C tarafına `erk_element_create`, `erk_text_create`, `erk_node_insert_before`, `erk_node_remove`, `erk_node_set_attr` fonksiyonları (ABI v0.3) eklendi.
   * Sınıf manipülasyonu için `add_class`, `remove_class`, `has_class` yardımcıları hazırlandı.

---

### 💬 Sonuç

* **M3'te Kalan İş:** Sıfır. M3 tüm gereksinimleriyle tamamlandı.
* **M4 Durumu:** M4'e geçmek sadece mantıklı değil, proje fiilen M4'e geçmiş ve M4.0 commit'ini başarıyla tamamlamış durumda. 
* Proje sıradaki adımlar olan **M4.1 (Toplu değişiklikler & klavye olayları)** ve **M4.3 (Gradyanlar)** için tamamen hazırdır.
