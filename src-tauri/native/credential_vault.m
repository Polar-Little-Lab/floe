#import <Foundation/Foundation.h>
#import <Security/Security.h>
#import <LocalAuthentication/LocalAuthentication.h>
#include <stdint.h>
#include <string.h>

// All calls are synchronous: callers must use a blocking worker, never AppKit's
// main thread. No legacy-keychain fallback, iCloud synchronization or auth UI.
static NSMutableDictionary *query(const char *service) {
    if (!service) return nil;
    NSString *name = [NSString stringWithUTF8String:service];
    if (!name || ![name hasPrefix:@"dev.branchdesk.https.v1."] || name.length != @"dev.branchdesk.https.v1.".length + 64)
        return nil;
    LAContext *context = [LAContext new];
    context.interactionNotAllowed = YES;
    return [@{(__bridge id)kSecClass: (__bridge id)kSecClassGenericPassword,
              (__bridge id)kSecAttrService: name,
              (__bridge id)kSecAttrAccount: @"credential-v1",
              (__bridge id)kSecAttrSynchronizable: @NO,
              (__bridge id)kSecUseDataProtectionKeychain: @YES,
              (__bridge id)kSecUseAuthenticationContext: context} mutableCopy];
}

int32_t branchdesk_vault_put(const char *service, const uint8_t *bytes, size_t length) {
    @autoreleasepool {
        NSMutableDictionary *q = query(service);
        if (!q || !bytes || length == 0 || length > 16384) return errSecParam;
        // Borrow Rust's zeroizing buffer; Foundation does not own or copy it.
        NSData *data = [NSData dataWithBytesNoCopy:(void *)bytes length:length freeWhenDone:NO];
        NSDictionary *changes = @{(__bridge id)kSecValueData: data};
        OSStatus status = SecItemUpdate((__bridge CFDictionaryRef)q, (__bridge CFDictionaryRef)changes);
        if (status != errSecItemNotFound) return status;
        q[(__bridge id)kSecValueData] = data;
        q[(__bridge id)kSecAttrAccessible] = (__bridge id)kSecAttrAccessibleWhenUnlockedThisDeviceOnly;
        // A competing creation returns duplicate; never delete an existing item.
        return SecItemAdd((__bridge CFDictionaryRef)q, NULL);
    }
}

int32_t branchdesk_vault_get(const char *service, uint8_t *output, size_t capacity, size_t *length) {
    @autoreleasepool {
        if (!output || !length) return errSecParam;
        *length = 0;
        NSMutableDictionary *q = query(service);
        if (!q) return errSecParam;
        q[(__bridge id)kSecReturnData] = @YES;
        q[(__bridge id)kSecMatchLimit] = (__bridge id)kSecMatchLimitOne;
        CFTypeRef result = NULL;
        OSStatus status = SecItemCopyMatching((__bridge CFDictionaryRef)q, &result);
        if (status != errSecSuccess) { if (result) CFRelease(result); return status; }
        if (!result || CFGetTypeID(result) != CFDataGetTypeID()) {
            if (result) CFRelease(result);
            return errSecDecode;
        }
        CFDataRef data = (CFDataRef)result;
        CFIndex count = CFDataGetLength(data);
        if (count <= 0 || (size_t)count > capacity) {
            CFRelease(result);
            return errSecDecode;
        }
        memcpy(output, CFDataGetBytePtr(data), (size_t)count);
        *length = (size_t)count;
        CFRelease(result);
        return errSecSuccess;
    }
}

int32_t branchdesk_vault_delete(const char *service) {
    @autoreleasepool {
        NSMutableDictionary *q = query(service);
        if (!q) return errSecParam;
        return SecItemDelete((__bridge CFDictionaryRef)q);
    }
}
