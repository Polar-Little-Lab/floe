#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <stdbool.h>

static bool (*request_save)(void) = NULL;
static BOOL waiting_for_save = NO;

static NSApplicationTerminateReply should_terminate(id self, SEL command, NSApplication *sender) {
    (void)self; (void)command; (void)sender;
    if (waiting_for_save) return NSTerminateLater;
    if (request_save && request_save()) {
        waiting_for_save = YES;
        return NSTerminateLater;
    }
    return NSTerminateNow;
}

// Extend only the optional method absent in the current Tao delegate; do not
// replace an existing implementation or change the delegate's class/ownership.
int branchdesk_install_termination(bool (*callback)(void)) {
    if (![NSThread isMainThread] || callback == NULL || NSApp.delegate == nil) return 1;
    Class cls = object_getClass(NSApp.delegate);
    SEL selector = @selector(applicationShouldTerminate:);
    if (class_getInstanceMethod(cls, selector) != NULL) return 2;
    request_save = callback;
    const char *encoding = @encode(NSUInteger);
    char signature[16];
    snprintf(signature, sizeof(signature), "%s@:@", encoding);
    if (!class_addMethod(cls, selector, (IMP)should_terminate, signature)) {
        request_save = NULL;
        return 3;
    }
    return 0;
}

// Return whether this completed an outstanding AppKit termination request.
bool branchdesk_reply_termination(bool saved) {
    if (![NSThread isMainThread] || !waiting_for_save) return false;
    waiting_for_save = NO;
    [NSApp replyToApplicationShouldTerminate:saved ? YES : NO];
    return true;
}
