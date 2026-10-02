# 📝 Erk Fonts (M1/system fonts) - Kod İnceleme Raporu

**Commit:** `35ef6a7358deba3eaba3be52d6f665461077b648`
**Değişiklik:** Sistem fontlarının (işletim sistemine ait font dosyalarının) okunması host'a (shell'e) devredildi. Core (renderer), host'tan font kataloğunu (kategori ve script'e göre fallback listelerini) alıyor ve ihtiyaç duyduğunda font verilerini `font:` şemalı URL'lerle Resource API üzerinden talep ediyor. CJK, Arabic vb. diller için fallback sistemi eklendi.

### 🌟 Artılar ve Çözülen Sorunlar
1. **İzolasyon (Sandboxing) Kontratına Tam Uyum:**
   * Disk IO işlemlerinin Core (renderer) içerisinden tamamen silinmesi ve işletim sisteminin Font API'lerine (DirectWrite, CoreText, Fontconfig vb.) erişimin Shell'e bırakılması muazzam bir mimari karar. Bu sayede Core platform-agnostic (platformdan bağımsız) ve güvenli (sandboxed) kalmaya devam ediyor.
   * `check-core-io.sh` script'i güncellenerek, yanlışlıkla bağımlılıklara `system` flag'i eklenip disk erişimi yapılmasının kalıcı olarak engellenmesi (CI tarafında) harika bir "savunma odaklı (defensive) programlama" örneğidir.
2. **Performans (Gecikmeli Yükleme - Lazy Loading):**
   * Bütün font dosyalarının başlangıçta belleğe okunması (startup delay'i 1.3 saniye civarıdır) engellenerek, host tarafında sadece metadata (FontCatalog) taraması yapılmasına ve *sadece* metin layout'unda (Parley) ihtiyaç duyulan font ailesinin (ve varyantlarının) talep edilmesine geçilmesi startup performansını kurtarmış.
3. **Manuel Percent-Decoding (URL) Ustalığı:**
   * `erk-shell/src/fonts.rs` içerisindeki `parse` fonksiyonunda URL unescape işlemi için yapılan ardışık `replace` zinciri (`%3F`, `%26`, `%23` ve **en son** `%25`) sıralama olarak çok zekice düşünülmüş. `%25`'in en son decode edilmesi "double-unescaping" (çift deşifre) hatalarını (Örn: `%2526` -> `%26` -> `&` yerine `%2526` -> `%26`) matematiksel olarak engelliyor. Mükemmel detay!
4. **Fallback ve Script İşlemleri:**
   * Her dil (Japonca, İbranice, Arapça vs.) ve senaryo (Emoji) için doğru font listelerinin çekilmesi ve listelerin sonuna mutlaka `Noto Sans` gibi bir kurtarıcının eklenmesi güvenilirliği (robustness) çok artırıyor. 

### ⚠️ Ufak Tavsiyeler (Gelecek İçin)
1. Commit notunuzda "A right-to-left paragraph takes its direction from its first letter rather than from CSS direction (Parley 0.11 has no setting)" olarak belirttiğiniz konu Parley'in kendi API eksikliğinden kaynaklandığı için şu aşamada motor tarafında yapılacak bir şey görünmüyor. Ancak Parley güncellendiğinde CSS `direction` desteğini entegre etmek iyi bir `TODO` olabilir.

### 💬 Sonuç
Hem uluslararası font desteklerinin CJK/Arabic dillerine kadar genişletilmesi hem de Core-Shell arası izolasyonun Resource API üzerinden başarılı bir şekilde devam ettirilmesi harika işlenmiş. P1 Kontratına (p1-contract) tam uyan pırıl pırıl bir geliştirme. Main branch için kesinlikle uygundur!
