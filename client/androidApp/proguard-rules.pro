-keepattributes RuntimeVisibleAnnotations,RuntimeVisibleParameterAnnotations,AnnotationDefault,Signature,InnerClasses,EnclosingMethod

# kotlinx.serialization resolves serializers through synthetic companions and $serializer
# classes that nothing references by name, so R8 full mode strips them without these.
-if @kotlinx.serialization.Serializable class **
-keepclassmembers class <1> {
    static <1>$Companion Companion;
}
-if @kotlinx.serialization.Serializable class ** {
    static **$* *;
}
-keepclassmembers class <2>$<3> {
    kotlinx.serialization.KSerializer serializer(...);
}
-if @kotlinx.serialization.Serializable class ** {
    public static ** INSTANCE;
}
-keepclassmembers class <1> {
    public static <1> INSTANCE;
    kotlinx.serialization.KSerializer serializer(...);
}
-keepclassmembers class **$$serializer {
    *** descriptor;
    *** childSerializers(...);
    *** typeParametersSerializers(...);
}
-dontnote kotlinx.serialization.**

# Ktor picks engines and plugins out of ServiceLoader/attribute keys.
-keep class io.ktor.client.engine.okhttp.OkHttpEngineContainer { *; }
-keep,allowobfuscation class io.ktor.serialization.kotlinx.** { *; }
-dontwarn io.ktor.**
-dontwarn org.slf4j.**
-dontwarn kotlinx.atomicfu.**
-dontwarn org.conscrypt.**
-dontwarn org.bouncycastle.**
-dontwarn org.openjsse.**
-dontwarn okhttp3.internal.platform.**
-keepnames class okhttp3.internal.publicsuffix.PublicSuffixDatabase

# Compose Multiplatform resources are addressed by asset path from generated accessors;
# the readers themselves are reached reflectively per platform (probe #64's blast radius).
-keep class org.jetbrains.compose.resources.** { *; }
-keep class dev.kampr.shared.res.** { *; }
-dontwarn org.jetbrains.compose.resources.**

# Credential Manager loads its Play-services provider by class name out of a resource string, so
# R8 sees nothing referencing it and strips the only authenticator on the device.
-keep class androidx.credentials.playservices.** { *; }
