#import <AppKit/AppKit.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>

@interface FLOECredentialForm : NSObject
@property(strong) NSPopUpButton *mode;
@property(strong) NSTextField *username;
@property(strong) NSSecureTextField *password;
@property(strong) NSButton *remember;
- (void)changed:(id)sender;
@end
@implementation FLOECredentialForm
- (void)changed:(id)sender {
    (void)sender;
    BOOL entering = self.mode.indexOfSelectedItem == 1;
    self.username.enabled = entering;
    self.password.enabled = entering;
    self.remember.enabled = entering;
    if (!entering) {
        self.username.stringValue = @"";
        self.password.stringValue = @"";
        self.remember.state = NSControlStateValueOff;
    }
}
@end

// Only the AppKit main thread may enter. Secret bytes return through a caller-
// owned native buffer, never an event, renderer callback, file or pasteboard.
int32_t branchdesk_prompt_credential(const char *endpoint, const char *title, const char *message,
    uint8_t *output, size_t capacity, size_t *length, int32_t *mode, bool *remember) {
    @autoreleasepool {
        if (![NSThread isMainThread] || NSApp == nil || !endpoint || !title || !message ||
            !output || capacity < 16384 || !length || !mode || !remember) return -1;
        *length = 0; *mode = 0; *remember = false;
        @try {
            NSString *destination = [NSString stringWithUTF8String:endpoint];
            NSString *heading = [NSString stringWithUTF8String:title];
            NSString *detail = [NSString stringWithUTF8String:message];
            if (!destination || !heading || !detail) return -1;
            NSAlert *alert = [NSAlert new];
            alert.messageText = heading;
            alert.informativeText = @"Review this operation and choose authentication for the exact URL below.";
            [alert addButtonWithTitle:@"Continue"];
            [alert addButtonWithTitle:@"Cancel"];
            NSView *view = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 500, 330)];
            NSTextField *address = [[NSTextField alloc] initWithFrame:NSMakeRect(0, 296, 500, 26)];
            address.stringValue = destination;
            address.editable = NO; address.selectable = YES;
            address.accessibilityLabel = @"Exact HTTPS credential destination";
            address.toolTip = destination;
            [view addSubview:address];
            NSScrollView *details = [[NSScrollView alloc] initWithFrame:NSMakeRect(0, 184, 500, 100)];
            details.hasVerticalScroller = YES;
            NSTextView *operation = [[NSTextView alloc] initWithFrame:NSMakeRect(0, 0, 480, 100)];
            operation.editable = NO; operation.selectable = YES; operation.richText = NO;
            operation.verticallyResizable = YES; operation.horizontallyResizable = NO;
            operation.autoresizingMask = NSViewWidthSizable;
            operation.textContainer.widthTracksTextView = YES;
            operation.textContainer.containerSize = NSMakeSize(480, CGFLOAT_MAX);
            operation.string = detail;
            operation.accessibilityLabel = @"Requested Git operation";
            details.documentView = operation;
            [view addSubview:details];
            FLOECredentialForm *form = [FLOECredentialForm new];
            form.mode = [[NSPopUpButton alloc] initWithFrame:NSMakeRect(0, 146, 500, 28) pullsDown:NO];
            [form.mode addItemsWithTitles:@[@"Anonymous — send no credentials", @"Enter username and token", @"Use saved Keychain credential for this URL"]];
            form.mode.accessibilityLabel = @"Authentication method";
            form.mode.target = form; form.mode.action = @selector(changed:);
            [view addSubview:form.mode];
            form.username = [[NSTextField alloc] initWithFrame:NSMakeRect(0, 110, 500, 26)];
            form.username.placeholderString = @"Username";
            form.username.accessibilityLabel = @"HTTPS username";
            [view addSubview:form.username];
            form.password = [[NSSecureTextField alloc] initWithFrame:NSMakeRect(0, 74, 500, 26)];
            form.password.placeholderString = @"Personal access token or password";
            form.password.accessibilityLabel = @"HTTPS token or password";
            [view addSubview:form.password];
            form.remember = [NSButton checkboxWithTitle:@"Save in Keychain for this exact URL (replaces saved login)" target:nil action:nil];
            form.remember.frame = NSMakeRect(0, 40, 500, 24);
            [view addSubview:form.remember];
            NSTextField *hint = [NSTextField labelWithString:@"Unchecked: use once. Tokens never enter the web page."];
            hint.frame = NSMakeRect(0, 4, 500, 28);
            [view addSubview:hint];
            [form changed:nil];
            alert.accessoryView = view;
            while (true) {
                NSModalResponse response = [alert runModal];
                if (response != NSAlertFirstButtonReturn) {
                    form.username.stringValue = @""; form.password.stringValue = @"";
                    return 0;
                }
                NSInteger selection = form.mode.indexOfSelectedItem;
                if (selection == 0 || selection == 2) { *mode = (int32_t)selection; return 1; }
                if (selection != 1) return -1;
                NSString *user = form.username.stringValue;
                NSString *secret = form.password.stringValue;
                NSData *userData = [user dataUsingEncoding:NSUTF8StringEncoding];
                NSData *secretData = [secret dataUsingEncoding:NSUTF8StringEncoding];
                NSCharacterSet *control = NSCharacterSet.controlCharacterSet;
                if (!userData.length || userData.length > 1024 || !secretData.length || secretData.length > 8192 ||
                    [user containsString:@":"] || [user rangeOfCharacterFromSet:control].location != NSNotFound ||
                    [secret rangeOfCharacterFromSet:control].location != NSNotFound) {
                    hint.stringValue = @"Enter a valid username and token. Control characters are not allowed.";
                    continue;
                }
                size_t count = userData.length;
                memcpy(output, "BDC1", 4);
                output[4] = (uint8_t)(count >> 24); output[5] = (uint8_t)(count >> 16);
                output[6] = (uint8_t)(count >> 8); output[7] = (uint8_t)count;
                memcpy(output + 8, userData.bytes, count);
                memcpy(output + 8 + count, secretData.bytes, secretData.length);
                *length = 8 + count + secretData.length;
                *mode = 1; *remember = form.remember.state == NSControlStateValueOn;
                form.username.stringValue = @""; form.password.stringValue = @"";
                return 1;
            }
        } @catch (NSException *exception) {
            (void)exception;
            // No exception text: native controls can contain secrets.
            memset(output, 0, capacity); *length = 0;
            return -2;
        }
    }
}
