export type LfsDownloadResult='created'|'alreadyPresent'|'cancelled'|null;
export type LfsUploadResult='uploaded'|'uploadedVerified'|'remotePresent'|'cancelled'|null;
export type LfsTransferResult=LfsDownloadResult|LfsUploadResult;
export type LfsTransferMode='download'|'upload';
export interface LfsUploadObservation {present:boolean;checkedAt:number}
export interface LfsUploadRecord {observation?:LfsUploadObservation;id:string;oid:string;size:string;endpoint:string;requiresVerify:boolean;status:'pending'|'accepted'|'verified'|'notSent'|'reviewed'}
export interface LfsIndexPage {entries:{path:string;text:string}[];offset:number;nextOffset:number|null;totalIndexEntries:number;skippedNonUtf8:number}
export interface LfsCheckoutPreview {path:string;indexOid:string;oid:string;size:string;executable:boolean;worktreeState:'missing'|'pointer'|'different'|'unsupported';cache:{state:string;message:string}}
